#[cfg(test)]
mod tests {
    use std::{collections::HashMap, process::Command, thread};

    use assert_cmd::{prelude::*, cargo::cargo_bin};
    use reqwest::StatusCode;
    use shared::{generate_example_tasks, MsgTaskRequest, MsgTaskResult, MsgId, MyUuid};


    #[test]
    fn integration_test() { // TODO: Split into several tests (how?)
        let (tx, rx) = tokio::sync::oneshot::channel::<&str>();
        let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME")).unwrap();
        let mut process = cmd.spawn()
            .map_err(|e| { panic!("Unable to start server process: {}", e) } )
            .unwrap();

        thread::spawn(move || {
            println!("Server started.");
            if let Err(e) = rx.blocking_recv() {
                println!("Server kill errored: {}", e);
            }
            println!("Received request to kill server.");
            if process.kill().is_ok() {
                println!("Server killed.");
            }
        });

        let mut examples = generate_example_tasks();
        let client = reqwest::blocking::Client::new();

        for task in examples.values_mut() {
            // POST /tasks
            let resp =
                client.post(format!("http://localhost:8080/tasks?worker_id={}", task.to[0]))
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
            // println!("Received Task ID: {}", servergenerated_task_id);

            // GET /tasks
            let resp =
                client.get(format!("http://localhost:8080/tasks?worker_id={}", task.to[0]))
                .json(&task)
                .send();
            assert!(resp.is_ok());
            let resp = resp.unwrap();
            // println!("Response: {:?}", resp);
            assert!(resp.status() == StatusCode::OK);
            let tasks = resp.json::<Vec<MsgTaskRequest>>().unwrap();
            assert!(tasks.len() == 1);

            // POST /tasks/.../results
            let mut successful_results = 0;
            for result in task.results.values_mut() {
                result.task = servergenerated_task_id;
                // println!("==> Reporting result for task {}, worker {}", servergenerated_task_id, result.worker_id);
                let resp = 
                    client.post(format!("http://localhost:8080/tasks/{}/results", servergenerated_task_id))
                    .json(&result)
                    .send();
                // println!("Response: {:?}", resp);
                assert!(resp.is_ok());
                if task.to.contains(&result.worker_id) {
                    assert!(resp.unwrap().error_for_status().is_ok());
                    successful_results = successful_results + 1;
                } else {
                    assert!(resp.unwrap().status() == StatusCode::UNAUTHORIZED);
                }
            }

            // GET /tasks/.../results (this time, let's wait for a short amount of time using long-polling)
            let resp = 
                client.get(format!("http://localhost:8080/tasks/{}/results?poll_count=2&poll_timeout=10", servergenerated_task_id))
                .send();
            assert!(resp.is_ok());
            let fetched_results: Vec<MsgTaskResult> = resp.unwrap().json().unwrap();
            // println!("Results: {:?}", fetched_results);
            assert!(fetched_results.len() == successful_results);
        }
        tx.send("Kill server, please!")
            .expect("Unable to request to stop server");
    }
}