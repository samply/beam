use shared::examples::generate_example_tasks;

#[cfg(debug_assertions)]
pub(crate) fn print_example_objects() -> bool {
    use shared::beam_id2::{BrokerId, BeamId};

    if std::env::args().nth(1).unwrap_or_default() == "examples" {
        let broker_id = match std::env::args().nth(2) {
            Some(id) => BrokerId::new(&id).ok(),
            None => todo!(),
        };
        let proxy_id = match std::env::args().nth(3) {
            Some(id) => BrokerId::new(&id).ok(),
            None => todo!(),
        };
        let tasks = generate_example_tasks(broker_id);
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