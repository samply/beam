use shared::generate_example_tasks;

#[cfg(debug_assertions)]
pub(crate) fn print_example_objects() -> bool {
    use shared::ClientId;

    if std::env::args().nth(1).unwrap_or_default() == "examples" {
        let client_id = match std::env::args().nth(2) {
            Some(id) => ClientId::new(&id).ok(),
            None => todo!(),
        };
        let tasks = generate_example_tasks(client_id);
        let mut num_results = 0;
        for (num_tasks, task) in tasks.values().enumerate() {
            println!("export TASK{}='{}'", num_tasks, serde_json::to_string(task).unwrap().replace('\'', "\'"));
            for result in task.results.values() {
                println!("export RESULT{}='{}'", num_results, serde_json::to_string(result).unwrap().replace('\'', "\'"));
                num_results += 1;
            }
        }
        true
    } else {
        false
    }
}