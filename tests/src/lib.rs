#![allow(unused_imports)]

#[cfg(test)]
mod tests {
    use std::{process::{Command, Child}, thread, time::{Duration, Instant}, io::ErrorKind, collections::{HashSet, HashMap}, iter::Map, path::Path, mem::take};

    use assert_cmd::prelude::*;
    use regex::Regex;
    use reqwest::{StatusCode, header::{self, HeaderValue, AUTHORIZATION}, Method};
    use restest::{request, Context, Request, assert_body_matches};
    use shared::{generate_example_tasks, MsgTaskRequest, MsgTaskResult, MyUuid, ClientId, Msg, MsgId};
    use static_init::dynamic;

    use rsa::{pkcs8::DecodePrivateKey, pkcs1::DecodeRsaPrivateKey};
    use tokio::{sync::{oneshot, OnceCell, Mutex}, runtime::Handle};

    const PEMFILE: &str = "../pki/test.priv.pem";
    const MY_PROXY_ID: &str = "test.broker.samply.de";
    const MY_CLIENT_ID_SHORT: &str = "itsme";
    const VAULT_BASE: &str = "http://localhost:8200";
    const VAULT_HEALTH: &str = "http://localhost:8200/v1/sys/health";
    const PROXY_HEALTH: &str = "http://localhost:8081/v1/health";
    const CENTRAL_HEALTH: &str = "http://localhost:8080/v1/health";
    // const CTX_PROXY: Context = Context::new().with_port(8081);
    // const CTX_VAULT: Context = Context::new().with_port(8200);

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
            for (cmd, args, env, wait_pkcs1_key, _wait_health) in 
                [
                    ("bash",
                        vec!("-c", "../pki/pki devsetup"),
                        HashMap::new(),
                        Some(PEMFILE),
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
                println!("{}: START", cmd);
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
                println!("{}: STARTED", cmd);
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
    static mut EXAMPLES: HashMap<MyUuid, MsgTaskRequest> = generate_example_tasks(Some(ClientId::new(MY_PROXY_ID).unwrap()));

    #[dynamic]
    static MY_CLIENT_ID: ClientId = ClientId::new(MY_PROXY_ID).unwrap();
    // #[dynamic]
    // static mut EXAMPLES: HashMap<MyUuid, MsgTaskRequest> = generate_example_tasks(Some(ClientId::new(MY_PROXY_ID).unwrap()));

    #[dynamic]
    static CLIENT: reqwest::Client = {
        let mut headers = header::HeaderMap::new();
        let mut value = header::HeaderValue::from_str(AUTH.as_str()).unwrap();
        value.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, value);
        reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap()
    };
    
    #[tokio::test]
    async fn b_post_task() -> anyhow::Result<()>{
        wait_for_servers().await;
        for task in EXAMPLES.fast_write().unwrap().values_mut() {
            let resp = CLIENT
                .post("http://localhost:8081/v1/tasks")
                .json(task)
                .send().await?;
            assert_eq!(resp.status(), StatusCode::CREATED);
            let location = resp.headers().get(header::LOCATION)
                .expect("Returned Location header is empty")
                .to_str()?;
            let location_regex = Regex::new("^.*/(.*)/?$")?;
            println!("Location: {}", location);
            let cap = location_regex.captures(location)
                .expect("Returned Location header does not match format.");
            let location = &cap[1];
            task.id = MsgId::try_from(location)?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn c_post_results() -> anyhow::Result<()> {
        let mut ex = EXAMPLES.fast_write().unwrap();
        for task in ex.values_mut() {
            for (_, mut result) in &mut task.results {
                result.msg.task = task.id;
                result.msg.from = MY_CLIENT_ID.clone();
                let resp = CLIENT
                    .post(format!("http://localhost:8081/v1/tasks/{}/results", task.id))
                    .json(&result.msg)
                    .send().await?;
                assert!([StatusCode::CREATED, StatusCode::NO_CONTENT].contains(&resp.status()));
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn d_get_tasks() -> anyhow::Result<()>{
        let ex = EXAMPLES.fast_write().unwrap();
        for task in ex.values() {
            let resp =
                CLIENT.get(format!("http://localhost:8081/v1/tasks?from={}", task.from))
                .send().await?;
            assert_eq!(resp.status(), StatusCode::OK);
            let got_tasks = resp.json::<Vec<MsgTaskRequest>>().await.unwrap();
            // println!("Task: {:?}", task);
            // println!("Tasks: {:?}", tasks);
            assert_eq!(got_tasks.len(), ex.len());
        }
        Ok(())
    }
}
