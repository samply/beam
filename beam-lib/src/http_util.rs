use std::{time::Duration, future::Future, pin::Pin};

use reqwest::{Client, header::{self, HeaderValue, HeaderName}, Url, StatusCode, Response};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

use crate::{AddressingId, TaskRequest, MsgId, TaskResult, ProxyId};
#[cfg(feature = "sockets")]
use crate::SocketTask;

/// A client used for communicating with the beam network
#[derive(Debug, Clone)]
pub struct BeamClient {
    client: Client,
    beam_proxy_url: Url,
}

#[derive(Error, Debug)]
pub enum BeamError {
    #[error("Communication with beam proxy failed: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Unexpected status code {0}")]
    UnexpectedStatus(StatusCode),
    #[error("The following receivers had invalid certificates which is why the request has been canceld: {0:?}")]
    InvalidReceivers(Vec<ProxyId>),
    #[error("Other handler specific error: {0}")]
    Other(Box<dyn std::error::Error + Send + Sync>)
}

impl BeamError {
    fn other<T: Into<Box<dyn std::error::Error + Send + Sync + 'static>>>(e: T) -> Self {
        Self::Other(e.into())
    }
}

pub type Result<T> = std::result::Result<T, BeamError>;

/// Long polling blocking options
/// 
/// The behavior works as follows:
/// - `wait_count` and `wait_time` are unset => Don't wait
/// - `wait_count` is set and `wait_time` is unset => Wait for this many items to be ready
/// - `wait_count` is unset and `wait_time` is set => Wait for the given duration
/// - `wait_count` is set and `wait_time` is set => Stop waiting after either of the conditions is fulfilled
#[derive(Debug)]
pub struct BlockingOptions {
    /// How long to poll for
    pub wait_time: Option<Duration>,
    /// The number of elements to wait for
    pub wait_count: Option<u16>,
}

impl BlockingOptions {
    pub fn from_count(count: u16) -> Self {
        Self { wait_time: None, wait_count: Some(count) }
    }

    pub fn from_time(time: Duration) -> Self {
        Self { wait_time: Some(time), wait_count: None }
    }

    fn to_query(&self) -> String {
        match (self.wait_count, self.wait_time) {
            (None, None) => String::with_capacity(0),
            (None, Some(wt)) => format!("wait_time={}s", wt.as_secs()),
            (Some(count), None) => format!("wait_count={count}"),
            (Some(count), Some(wt)) => format!("wait_time={}s&wait_count={count}", wt.as_secs()),
        }
    }
}

impl BeamClient {
    /// Create a beam client based on the Authorization credentials and the beam proxy url
    pub fn new(app: &AddressingId, key: &str, beam_proxy_url: Url) -> Self {
        let default_headers = [
            (header::AUTHORIZATION, HeaderValue::from_bytes(format!("ApiKey {app} {key}").as_bytes()).expect("This is a valid header value"))
        ].into_iter().collect();
        Self {
            client: Client::builder().default_headers(default_headers).build().expect("Client should always build"),
            beam_proxy_url
        }
    }

    /// Construct a beam client from a [`reqwest::Client`]
    /// # Note:
    /// This expects the user to have set the default headers for authorization accordingly.
    /// If you don't need any special client configuration use [`BeamClient::new`].
    pub fn from_client(client: Client, beam_proxy_url: Url) -> Self {
        Self { client, beam_proxy_url }
    }

    /// Poll beam tasks using the `filter=todo` option and the given blocking options.
    /// The generic Parameter T represents the task body type that the requests are expected to have.
    pub async fn poll_pending_tasks<T: DeserializeOwned + 'static>(&self, blocking: &BlockingOptions) -> Result<Vec<TaskRequest<T>>> {
        let url = self.beam_proxy_url
            .join(&format!("/v1/tasks?filter=todo&{}", blocking.to_query()))
            .expect("The proxy url is valid");
        let response_result = self.client
            .get(url)
            .send()
            .await;
        let response = match response_result {
            Ok(res) => res,
            Err(e) => return if e.is_timeout() {
                Ok(Vec::with_capacity(0))
            } else {
                Err(e.into())
            },
        }.handle_invalid_receivers().await?;
        match response.status() {
            StatusCode::GATEWAY_TIMEOUT => Ok(Vec::with_capacity(0)),
            StatusCode::OK | StatusCode::PARTIAL_CONTENT => Ok(response.json().await?),
            status => Err(BeamError::UnexpectedStatus(status))
        }
    }

    /// Poll beam results for a given task id using the given blocking options.
    /// The generic Parameter T represents the result body type that the requests are expected to have.
    pub async fn poll_results<T: DeserializeOwned + 'static>(&self, task_id: &MsgId, blocking: &BlockingOptions) -> Result<Vec<TaskResult<T>>> {
        let url = self.beam_proxy_url
            .join(&format!("/v1/tasks/{task_id}/results?{}", blocking.to_query()))
            .expect("The proxy url is valid");
        let response_result = self.client
            .get(url)
            .send()
            .await;
        let response = match response_result {
            Ok(res) => res,
            Err(e) => return if e.is_timeout() {
                Ok(Vec::with_capacity(0))
            } else {
                Err(e.into())
            },
        }.handle_invalid_receivers().await?;
        match response.status() {
            // TODO: Add more status codes here
            StatusCode::GATEWAY_TIMEOUT => Ok(Vec::with_capacity(0)),
            StatusCode::OK | StatusCode::PARTIAL_CONTENT => Ok(response.json().await?),
            status => Err(BeamError::UnexpectedStatus(status))
        }
    }

    /// Post a beam task with a serializeable body.
    pub async fn post_task<T: Serialize + 'static>(&self, task: &TaskRequest<T>) -> Result<()> {
        let url = self.beam_proxy_url
            .join("/v1/tasks")
            .expect("The proxy url is valid");
        let response = self.client
            .post(url)
            .json(task)
            .send().await?
            .handle_invalid_receivers().await?;
        match response.status() {
            // TODO: Add more status codes here
            StatusCode::CREATED => Ok(()),
            status => Err(BeamError::UnexpectedStatus(status))
        }
    }

    /// Put a beam task result with a serializeable body.
    /// Returns true if the result was newly created or false if it was updated.
    pub async fn put_result<T: Serialize + 'static>(&self, result: &TaskResult<T>, for_task_id: &MsgId) -> Result<bool> {
        let url = self.beam_proxy_url
            .join(&format!("/v1/tasks/{for_task_id}/results/{}", result.from))
            .expect("The proxy url is valid");
        let response = self.client
            .put(url)
            .json(result)
            .send().await?
            .handle_invalid_receivers().await?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(false),
            StatusCode::CREATED => Ok(true),
            status => Err(BeamError::UnexpectedStatus(status))
        }
    }

    /// For low level beam request where full control of the request is required.
    /// This will return a [`reqwest::RequestBuilder`] with a url relative to the given path.
    pub fn raw_beam_request(&self, method: reqwest::Method, relative_path: &str) -> reqwest::RequestBuilder {
        let url = self.beam_proxy_url
            .join(relative_path)
            .expect("The proxy url is valid");
        self.client.request(method, url)
    }

    /// Create a socket task for some other application to connect to
    /// For this to work both the beam proxy and beam broker need to have the sockets feature enabled.
    #[cfg(feature = "sockets")]
    pub async fn create_socket(&self, destination: &AddressingId) -> Result<reqwest::Upgraded> {
        self.create_socket_with_metadata(destination, serde_json::Value::Null).await
    }

    /// Same as `create_socket` but with associated (unencrypted) metadata.
    #[cfg(feature = "sockets")]
    pub async fn create_socket_with_metadata(&self, destination: &AddressingId, metadata: impl Serialize) -> Result<reqwest::Upgraded> {
        const METADATA_HEADER: HeaderName = HeaderName::from_static("metadata");
        let url = self.beam_proxy_url
            .join(&format!("/v1/sockets/{destination}"))
            .expect("The proxy url is valid");
        let response = self.client
            .post(url)
            .header(header::UPGRADE, "tcp")
            .header(
                METADATA_HEADER,
                HeaderValue::try_from(serde_json::to_string(&metadata).map_err(BeamError::other)?).map_err(BeamError::other)?
            )
            .send().await?
            .handle_invalid_receivers().await?;
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            Err(BeamError::UnexpectedStatus(response.status()))
        } else {
            response
                .upgrade()
                .await
                .map_err(Into::into)
        }
    }

    /// Poll for socket tasks
    #[cfg(feature = "sockets")]
    pub async fn get_socket_tasks(&self, blocking: &BlockingOptions) -> Result<Vec<SocketTask>> {
        let url = self.beam_proxy_url
            .join(&format!("/v1/sockets?{}", blocking.to_query()))
            .expect("The proxy url is valid");
        let response_result = self.client
            .get(url)
            .send()
            .await;
        let response = match response_result {
            Ok(res) => res,
            Err(e) => return if e.is_timeout() {
                Ok(Vec::with_capacity(0))
            } else {
                Err(e.into())
            },
        }.handle_invalid_receivers().await?;
        match response.status() {
            // TODO: Add more status codes here
            StatusCode::GATEWAY_TIMEOUT => Ok(Vec::with_capacity(0)),
            StatusCode::OK | StatusCode::PARTIAL_CONTENT => Ok(response.json().await?),
            status => Err(BeamError::UnexpectedStatus(status))
        }
    }

    /// Connect to a socket by its socket task id
    #[cfg(feature = "sockets")]
    pub async fn connect_socket(&self, socket_task_id: &MsgId) -> Result<reqwest::Upgraded> {
        let url = self.beam_proxy_url
            .join(&format!("/v1/sockets/{socket_task_id}"))
            .expect("The proxy url is valid");
        let response = self.client
            .get(url)
            .header(header::UPGRADE, "tcp")
            .send().await?
            .handle_invalid_receivers().await?;
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            Err(BeamError::UnexpectedStatus(response.status()))
        } else {
            response
                .upgrade()
                .await
                .map_err(Into::into)
        }
    }
}

impl HandleInvalidReceiversExt for Response {
    fn handle_invalid_receivers(self) -> Pin<Box<dyn Future<Output = Result<Response>> + Send>> {
        async fn handle_invalid_receivers(res: Response) -> Result<Response> {
            if res.status() == StatusCode::FAILED_DEPENDENCY {
                Err(BeamError::InvalidReceivers(res.json().await?))
            } else {
                Ok(res)
            }
        }
        Box::pin(handle_invalid_receivers(self))
    }
}

trait HandleInvalidReceiversExt: Sized {
    fn handle_invalid_receivers(self) -> Pin<Box<dyn Future<Output = Result<Response>> + Send>>;
}
