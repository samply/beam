#[cfg(test)]
mod tests {
    use std::{process::{Command, Child}, thread, sync::mpsc::{self, Sender}, time::Duration, io::ErrorKind, collections::HashSet};

    use assert_cmd::prelude::*;
    use reqwest::{StatusCode, header};
    use shared::{generate_example_tasks, MsgTaskRequest, MsgTaskResult, MyUuid, ClientId, Msg};

    const PEMFILE: &str = "../pki/test.priv.pem";
    const MYID: &str = "test.broker.samply.de";

    struct Servers {
        txes: Vec<Sender<()>>
    }

    impl Servers {
        fn start() -> anyhow::Result<Self> {
            let mut txes = Vec::new();
            for (cmd, args, wait) in [
                    ("bash", vec!("-c", "../pki/pki devsetup"), None),
                    ("central", vec![], None),
                    ("proxy", vec!("--broker-url", "http://localhost:8080", "--client-id", MYID, "--pki-address", "http://localhost:8200", "--pki-secret-file", "broker.secrets.example", "--privkey-file", PEMFILE), Some(PEMFILE))
                    ] {
                let (tx, rx) = mpsc::channel::<()>();
                txes.push(tx);
                let mut proc = Command::cargo_bin(cmd)
                    .unwrap_or(Command::new(cmd));
                if let Some(wait) = wait {
                    while let Err(e) = std::fs::File::open(wait) {
                        if e.kind() == ErrorKind::NotFound {
                            thread::sleep(Duration::from_millis(100));
                        } else {
                            panic!("Waiting for file {} failed: {}", wait, e);
                        }
                    }
                    println!("Found file {}", wait);
                }
                let mut proc = proc
                    .args(args)
                    .spawn()
                    .expect(&format!("Unable to start process {}", cmd));
                thread::spawn(move || {
                    println!("Server {} started.", cmd);
                    if let Err(e) = rx.recv() {
                        println!("Server {} kill errored: {}", cmd, e);
                    }
                    println!("Received request to kill server {}.", cmd);
                    if proc.kill().is_ok() {
                        println!("Server {} killed.", cmd);
                    }
                });
            }
            Ok(Servers { txes })
        }

        fn stop_and_clean(&self) {
            for tx in self.txes.iter() {
                tx.send(());
            }
            for path in [glob::glob("../pki/*.pem").unwrap(), glob::glob("../pki/*.json").unwrap()].into_iter().flatten() {
                if let Ok(path) = path {
                    println!("Cleaning up: {}", path.display());
                    std::fs::remove_file(path);
                }
            }
        }
    }

    impl Drop for Servers {
        fn drop(&mut self) {
            self.stop_and_clean();
        }
    }

    #[test]
    fn all_servers_start_successfully() {
        let servers = Servers::start().unwrap();
    }

    #[test]
    #[should_panic]
    fn fails_without_apikey() {
        let servers = Servers::start().unwrap();
        integration_test(None).unwrap();
    }

    #[test]
    fn works_with_apikey() {
        let servers = Servers::start().unwrap();
        integration_test(Some("ClientApiKey EssenKey")).unwrap();
    }

    fn integration_test(apikey: Option<&'static str>) -> anyhow::Result<()> { // TODO: Split into several tests (how?)
        let mut headers = header::HeaderMap::new();
        if let Some(apikey) = apikey {
            headers.insert(header::PROXY_AUTHORIZATION, header::HeaderValue::from_static(apikey));
        }
        let client = reqwest::blocking::Client::builder()
            .default_headers(headers)
            .build()?;

        let mut examples = generate_example_tasks(Some(ClientId::new(MYID).unwrap()));

        thread::sleep(Duration::from_secs(1));

        let myid = ClientId::new(MYID).unwrap();

        for task in examples.values_mut() {
            task.from = myid.clone();
            // POST /tasks
            let resp =
                client.post(format!("http://localhost:8081/tasks"))
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
            assert!(location.contains("/tasks/"));
            let servergenerated_task_id = MyUuid::try_from(location.split("/").last().unwrap())
                .expect("Did not receive a correct Task ID.");

            // GET /tasks
            let resp =
                client.get(format!("http://localhost:8081/tasks?to={}", task.to[0]))
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
                        client.post(format!("http://localhost:8081/tasks/{}/results", servergenerated_task_id))
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
                client.get(format!("http://localhost:8081/tasks/{}/results?poll_count=2&poll_timeout=2", servergenerated_task_id))
                .send();
            assert!(resp.is_ok());
            let fetched_results: Vec<MsgTaskResult> = resp.unwrap().json().unwrap();
            assert_eq!(fetched_results.len(), unique_results);
        }
        Ok(())
    }
}