#![allow(unused_imports)]

#[cfg(test)]
mod tests {
    use std::{process::{Command, Child}, thread, time::{Duration, Instant}, io::ErrorKind, collections::{HashSet, HashMap}, iter::Map, path::Path, mem::take};

    use assert_cmd::prelude::*;
    use reqwest::{StatusCode, header::{self, HeaderValue, AUTHORIZATION}, Method};
    use restest::{request, Context, Request, assert_body_matches};
    use shared::{generate_example_tasks, MsgTaskRequest, MsgTaskResult, MyUuid, ClientId, Msg, MsgId};
    use lazy_static::lazy_static;
    use static_init::dynamic;

    use rsa::{pkcs8::FromPrivateKey, pkcs1::FromRsaPrivateKey};
    use tokio::{sync::{oneshot, OnceCell, Mutex}, runtime::Handle};

    const PEMFILE: &str = "../pki/test.priv.pem";
    const MY_PROXY_ID: &str = "test.broker.samply.de";
    const MY_CLIENT_ID_SHORT: &str = "itsme";
    const VAULT_BASE: &str = "http://localhost:8200";
    const VAULT_HEALTH: &str = "http://localhost:8200/v1/sys/health";
    const PROXY_HEALTH: &str = "http://localhost:8081/health";
    const CENTRAL_HEALTH: &str = "http://localhost:8080/health";
    const CTX_PROXY: Context = Context::new().with_port(8081);
    const CTX_VAULT: Context = Context::new().with_port(8200);

    #[dynamic(drop)]
    static mut SERVERS: Servers = Servers::start().unwrap();
    #[dynamic]
    static AUTH: String = {
        let mut a = String::from("ClientApiKey ");
        a.push_str(MY_CLIENT_ID_SHORT);
        a.push_str(".");
        a.push_str(MY_PROXY_ID);
        a.push_str(" MySecret");
        a
    };

    #[derive(Debug)]
    struct Servers {
        txes: Vec<oneshot::Sender<()>>,
    }

    impl Servers {
        fn start() -> anyhow::Result<Self> {
            let mut txes = Vec::new();
            for (cmd, args, env, wait_pkcs1_key, wait_health) in 
                [
                    ("bash",
                        vec!("-c", "../pki/pki devsetup"),
                        HashMap::new(),
                        None,
                        Some(VAULT_HEALTH)),
                    ("central",
                        vec!("--pki-address", VAULT_BASE, "--pki-apikey-file", "pki_apikey.secret", "--privkey-file", PEMFILE),
                        HashMap::new(),
                        Some(PEMFILE),
                        Some(CENTRAL_HEALTH)),
                    ("proxy", 
                        vec!("--broker-url", "http://localhost:8080", "--client-id", MY_PROXY_ID, "--pki-address", "http://localhost:8200", "--pki-apikey-file", "pki_apikey.secret", "--privkey-file", PEMFILE),
                        HashMap::from([("CLIENTKEY_".to_string() + MY_CLIENT_ID_SHORT, "MySecret")]),
                        Some(PEMFILE),
                        Some(PROXY_HEALTH))
                ] {
                let (tx, rx) = oneshot::channel::<()>();
                txes.push(tx);
                let mut proc = Command::cargo_bin(cmd)
                    .unwrap_or(Command::new(cmd));
                let proc = proc.envs(env);
                let mut proc = proc
                    .args(args)
                    .spawn()
                    .expect(&format!("Unable to start process {}", cmd));
                thread::spawn(move || {
                    println!("Server {} started.", cmd);
                    if let Err(e) = rx.blocking_recv() {
                        println!("Server {} kill errored: {}", cmd, e);
                    }
                    println!("Received request to kill server {}.", cmd);
                    if proc.kill().is_ok() {
                        println!("Server {} killed.", cmd);
                    }
                });
                // if let Some(wait) = wait_health {
                //     let started = Instant::now();
                //     while let Err(e) = reqwest::blocking::get(wait) {
                //         if started.elapsed() > Duration::from_secs(5) {
                //             panic!("Giving up after waiting {}s for a connection to {}.", started.elapsed().as_secs(), wait);
                //         }
                //         match e {
                //             // ...
                //             _ => {
                //                 thread::sleep(Duration::from_millis(100));
                //             },
                //         }
                //     }
                //     // println!("Service online at {}", wait);
                // }

                if let Some(wait) = wait_pkcs1_key {
                    let started = Instant::now();
                    while let Err(e) = rsa::RsaPrivateKey::read_pkcs1_pem_file(Path::new(wait)).or(rsa::RsaPrivateKey::read_pkcs8_pem_file(Path::new(wait))) {
                        if started.elapsed() > Duration::from_secs(30) {
                            panic!("Giving up after waiting {}s for a private key at {}.", started.elapsed().as_secs(), wait);
                        }
                        match e {
                            // ...
                            _ => {
                                thread::sleep(Duration::from_millis(100));
                            },
                        }
                    }
                    // println!("Found file {}", wait);
                }
            }
            Ok(Servers { txes })
        }

        async fn status() -> anyhow::Result<()> {
            // let handle = tokio::task::spawn_blocking(|| -> anyhow::Result<()>{
            for url in [VAULT_HEALTH, PROXY_HEALTH, CENTRAL_HEALTH] {
                Self::status_one(url).await?;
            }
            Ok(())
        }

        async fn status_one(url: &str) -> anyhow::Result<()> {
            let req = reqwest::get(url).await?;
            if req.status() != StatusCode::OK {
                req.error_for_status()?;
            }
            Ok(())
        }

        fn stop_and_clean(&mut self) {
            for tx in self.txes.drain(..) {
                tx.send(()).unwrap_or(());
            }
            for path in [glob::glob("../pki/*.pem").unwrap(), glob::glob("../pki/*.json").unwrap()].into_iter().flatten() {
                if let Ok(path) = path {
                    println!("Cleaning up: {}", path.display());
                    std::fs::remove_file(path).unwrap_or(());
                }
            }
        }
    }

    impl Drop for Servers {
        fn drop(&mut self) {
            self.stop_and_clean();
        }
    }

    #[tokio::test]
    async fn all_servers_start_successfully() {
        wait_for_servers().await;
    }

    async fn wait_for_servers() {
        while Servers::status().await.is_err() {
            println!("Servers not yet initialized.");
            tokio::task::yield_now().await;
        }
    }

    #[dynamic]
    static EXAMPLES: HashMap<MyUuid, MsgTaskRequest> = generate_example_tasks(Some(ClientId::new(MY_PROXY_ID).unwrap()));
    
    #[tokio::test]
    async fn b_post_task() {
        wait_for_servers().await;
        for (_ ,task) in EXAMPLES.iter() {
            let req = Request::post("/v1/tasks")
                .with_header(AUTHORIZATION, AUTH.as_str())
                .with_body(task);
            let body = CTX_PROXY.run(req)
                .await
                .expect_status_empty_body(StatusCode::CREATED)
                .await;
        }
    }

    fn integration_test(auth: Option<&str>) -> anyhow::Result<()> { // TODO: Split into several tests (how?)
        let mut headers = header::HeaderMap::new();
        if let Some(auth) = auth {
            let mut value = header::HeaderValue::from_str(auth).unwrap();
            value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, value);
        }
        let client = reqwest::blocking::Client::builder()
            .default_headers(headers)
            .build()?;

        let mut examples = generate_example_tasks(Some(ClientId::new(MY_PROXY_ID).unwrap()));

        let myid = ClientId::new(MY_PROXY_ID).unwrap();

        for task in examples.values_mut() {
            task.from = myid.clone();
            // POST /tasks
            let resp =
                client.post(format!("http://localhost:8081/v1/tasks"))
                .json(&task)
                .send();
            assert!(resp.is_ok());
            let resp = resp.unwrap();
            assert!(resp.status() == StatusCode::CREATED);
            let location = resp.headers().get(reqwest::header::LOCATION);
            assert!(location.is_some());
            let location = location.unwrap().to_str();
            assert!(location.is_ok());
            let location = location.unwrap();
            assert!(location.contains("/v1/tasks/"));
            let servergenerated_task_id = MyUuid::try_from(location.split("/").last().unwrap())
                .expect("Did not receive a correct Task ID.");

            // GET /tasks
            let resp =
                client.get(format!("http://localhost:8081/v1/tasks?to={}", task.to[0]))
                .send();
            assert!(resp.is_ok());
            let resp = resp.unwrap();
            assert!(resp.status() == StatusCode::OK);
            let tasks = resp.json::<Vec<MsgTaskRequest>>().unwrap();
            assert!(tasks.len() == 1);

            // POST /tasks/.../results
            let unique_results = { 
                let mut unique_results = HashSet::new();
                for result in task.results.values_mut() { // TODO: Remove clone()
                    result.msg.task = servergenerated_task_id;
                    result.msg.from = myid.clone();
                    let resp = 
                        client.post(format!("http://localhost:8081/v1/tasks/{}/results", servergenerated_task_id))
                        .json(&result.msg)
                        .send()?;
                    if task.to.contains(&result.get_from()) {
                        debug_assert!(resp.error_for_status().is_ok());
                        unique_results.insert(result.get_from());
                    } else {
                        assert!(resp.error_for_status().is_err());
                    }
                }
                unique_results.len()
            };

            // GET /tasks/.../results (this time, let's wait for a short amount of time using long-polling)
            let resp = 
                client.get(format!("http://localhost:8081/v1/tasks/{}/results?poll_count=2&poll_timeout=2", servergenerated_task_id))
                .send();
            assert!(resp.is_ok());
            let fetched_results: Vec<MsgTaskResult> = resp.unwrap().json().unwrap();
            assert_eq!(fetched_results.len(), unique_results);
        }
        Ok(())
    }
}
