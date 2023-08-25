use std::time::Duration;

use reqwest::{Client, header::{self, HeaderValue}, Url, StatusCode};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

use crate::{AddressingId, TaskRequest, MsgId, TaskResult};

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
    UnexpectedStatus(StatusCode)
}

pub type Result<T> = std::result::Result<T, BeamError>;

#[derive(Debug)]
pub struct BlockingOptions {
    pub wait_time: Option<Duration>,
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
    /// Note: This expects the user to have set the default headers for authorization accordingly
    /// If you don't need any special client configuration use [`BeamClient::new`]
    pub fn from_client(client: Client, beam_proxy_url: Url) -> Self {
        Self { client, beam_proxy_url }
    }

    pub async fn poll_pending_tasks<T: DeserializeOwned>(&self, blocking: &BlockingOptions) -> Result<Vec<TaskRequest<T>>> {
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
        };
        match response.status() {
            StatusCode::GATEWAY_TIMEOUT => Ok(Vec::with_capacity(0)),
            StatusCode::OK | StatusCode::PARTIAL_CONTENT => Ok(response.json().await?),
            status => Err(BeamError::UnexpectedStatus(status))
        }
    }

    pub async fn poll_results<T: DeserializeOwned>(&self, msg_id: &MsgId, blocking: &BlockingOptions) -> Result<Vec<TaskResult<T>>> {
        let url = self.beam_proxy_url
            .join(&format!("/v1/tasks/{msg_id}/results?{}", blocking.to_query()))
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
        };
        match response.status() {
            // TODO: Add more status codes here
            StatusCode::GATEWAY_TIMEOUT => Ok(Vec::with_capacity(0)),
            StatusCode::OK | StatusCode::PARTIAL_CONTENT => Ok(response.json().await?),
            status => Err(BeamError::UnexpectedStatus(status))
        }
    }

    pub async fn post_task<T: Serialize>(&self, task: &TaskRequest<T>) -> Result<()> {
        let url = self.beam_proxy_url
            .join("/v1/tasks")
            .expect("The proxy url is valid");
        let response_result = self.client
            .post(url)
            .json(task)
            .send()
            .await;
        match response_result?.status() {
            // TODO: Add more status codes here
            StatusCode::CREATED => Ok(()),
            status => Err(BeamError::UnexpectedStatus(status))
        }
    }

    pub async fn put_result<T: Serialize>(&self, result: &TaskResult<T>, for_task_id: &MsgId) -> Result<()> {
        let url = self.beam_proxy_url
            .join(&format!("/v1/tasks/{for_task_id}/results/{}", result.from))
            .expect("The proxy url is valid");
        let response_result = self.client
            .put(url)
            .json(result)
            .send()
            .await;
        match response_result?.status() {
            // TODO: Add more status codes here
            StatusCode::CREATED => Ok(()),
            status => Err(BeamError::UnexpectedStatus(status))
        }
    }
}
