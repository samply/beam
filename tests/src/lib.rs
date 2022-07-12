#![allow(unused_imports)]

#[cfg(test)]
mod tests {
    use std::{process::{Command, Child, Stdio}, thread, time::{Duration, Instant}, io::{ErrorKind, BufReader, BufRead}, collections::{HashSet, HashMap}, iter::Map, path::Path, mem::take};

    use assert_cmd::prelude::*;
    use regex::Regex;
    use reqwest::{StatusCode, header::{self, HeaderValue, AUTHORIZATION}, Method, Url};
    use shared::{MsgTaskRequest, MsgTaskResult, MyUuid, Msg, MsgId, config_proxy, beam_id::{AppId, BeamId, BrokerId, ProxyId}, examples::generate_example_tasks};
    use static_init::dynamic;

    use rsa::{pkcs8::DecodePrivateKey, pkcs1::DecodeRsaPrivateKey};
    use tokio::{sync::{oneshot, OnceCell, Mutex}, runtime::Handle};

    const BROKER_URL: &str = "http://localhost:8080";
    const PROXY_URL: &str = "http://localhost:8081";
    const PROXY_ID_SHORT: &str = "proxy23";
    const APP_ID_SHORT: &str = "app1";
    const VAULT_BASE: &str = "http://localhost:8200";
    const VAULT_HEALTH: &str = "http://localhost:8200/v1/sys/health";
    const PROXY_HEALTH: &str = "http://localhost:8081/v1/health";
    const CENTRAL_HEALTH: &str = "http://localhost:8080/v1/health";
    // const CTX_PROXY: Context = Context::new().with_port(8081);
    // const CTX_VAULT: Context = Context::new().with_port(8200);

    #[dynamic]
    static BROKER_ID: String = Url::try_from(BROKER_URL).unwrap().host_str().unwrap().to_string();
    #[dynamic]
    static PROXY_ID: String = format!("{PROXY_ID_SHORT}.{}", BROKER_ID.as_str());
    #[dynamic(lazy)]
    static APP_ID: AppId = AppId::new(&format!("{}.{}", APP_ID_SHORT, PROXY_ID.as_str())).unwrap();

    #[dynamic(lazy)]
    static mut EXAMPLES: (Vec<MsgTaskRequest>,Vec<MsgTaskResult>) = generate_example_tasks(Some(BrokerId::new(&BROKER_ID).unwrap()), Some(ProxyId::new(PROXY_ID.as_str()).unwrap()));

    #[dynamic(drop)]
    static mut SERVERS: Servers = Servers::start().unwrap();
    #[dynamic]
    static AUTH: String = {
        let mut a = String::from("ApiKey ");
        a.push_str(APP_ID_SHORT);
        a.push('.');
        a.push_str(PROXY_ID.as_str());
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
            let pem_file = &format!("../pki/{}.priv.pem", PROXY_ID_SHORT);
            for (cmd, args, env, wait_pkcs1_key, _wait_health, wait_output) in 
                [
                    ("bash",
                        vec!("-c", "../pki/pki devsetup"),
                        HashMap::new(),
                        Some(pem_file),
                        Some(VAULT_HEALTH),
                        Some("Success: PEM files stored to")),
                    ("broker",
                        vec!("--broker-url", BROKER_URL, "--pki-address", VAULT_BASE, "--pki-apikey-file", "pki_apikey.secret", "--privkey-file", pem_file),
                        HashMap::new(),
                        Some(pem_file),
                        Some(CENTRAL_HEALTH),
                        None,),
                    ("proxy", 
                        vec!("--broker-url", BROKER_URL, "--proxy-id", PROXY_ID.as_str(), "--pki-address", VAULT_BASE, "--pki-apikey-file", "pki_apikey.secret", "--privkey-file", pem_file),
                        HashMap::from([(config_proxy::APPKEY_PREFIX.to_string() + APP_ID_SHORT, "MySecret")]),
                        Some(pem_file),
                        Some(PROXY_HEALTH),
                        None)
                ] {
                println!("{}: START with args: {:?}", cmd, args);
                let (tx, rx) = oneshot::channel::<()>();
                txes.push(tx);
                let mut proc = Command::cargo_bin(cmd)
                    .unwrap_or(Command::new(cmd));
                println!("Command: {:?}", proc);
                let mut proc = {
                    let mut proc = proc
                        .envs(env)
                        .args(args);
                    if wait_output.is_some() {
                        proc = proc.stdout(Stdio::piped());
                    }
                    proc
                        .spawn()
                        .expect(&format!("Unable to start process {}", cmd))
                };
                if let Some(wait_output) = wait_output {
                    let mut buf = String::new();
                    let mut reader = BufReader::new(proc.stdout.take().unwrap());
                    while reader.read_line(&mut buf).unwrap() > 0 {
                        if buf.contains(wait_output) {
                            eprintln!("Awaitet output received: \"{wait_output}\"");
                            break;
                        }
                        buf.clear();
                    }
                }
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
        for task in &EXAMPLES.fast_read().unwrap().0 {
            let req = CLIENT
                .post("http://localhost:8081/v1/tasks")
                .json(task);
            let resp = req.send().await?;
            assert_eq!(resp.status(), StatusCode::CREATED, "\nSent: {:?}\nGot: {:?}", task, resp);
            let location = resp.headers().get(header::LOCATION)
                .expect("Returned Location header is empty")
                .to_str()?;
            let location_regex = Regex::new("^.*/(.*)/?$")?;
            println!("Location: {}", location);
            location_regex.captures(location)
                .expect("Returned Location header does not match format.");
            assert!(location.contains(&task.id.to_string()));
        }
        Ok(())
    }

    #[tokio::test]
    async fn c_post_results() -> anyhow::Result<()> {
        for result in &EXAMPLES.fast_read().unwrap().1 {
            // result.msg.task = task.id;
            // result.msg.from = APP_ID.clone().into();
            let resp = CLIENT
                .post(format!("http://localhost:8081/v1/tasks/{}/results", result.task))
                .json(&result)
                .send().await?;
            if result.from == APP_ID.value() {
                assert!([StatusCode::CREATED, StatusCode::NO_CONTENT].contains(&resp.status()), "\nSent: {:?}\nGot: {:?}", result, resp);
            } else {
                assert!(resp.status() == StatusCode::UNAUTHORIZED);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn d_get_tasks() -> anyhow::Result<()>{
        let ex = &EXAMPLES.fast_read().unwrap().0;
        for task in ex {
            let resp =
                CLIENT.get(format!("{}/v1/tasks?from={}", PROXY_URL, task.from))
                .send().await?;
            assert_eq!(resp.status(), StatusCode::OK);
            let got_tasks = resp.json::<Vec<MsgTaskRequest>>().await.unwrap();
            assert_eq!(got_tasks.len(), ex.len());
        }
        Ok(())
    }
}
