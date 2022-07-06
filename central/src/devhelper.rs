use shared::generate_example_tasks;

#[cfg(debug_assertions)]
pub(crate) fn print_example_objects() -> bool {
    if std::env::args().nth(1).unwrap_or_default() == "examples" {
        let tasks = generate_example_tasks(None);
        let mut num_tasks = 0;
        let mut num_results = 0;
        for task in tasks.values() {
            println!("export TASK{}='{}'", num_tasks, serde_json::to_string(task).unwrap().replace("'", "\'"));
            num_tasks = num_tasks + 1;
            for result in task.results.values() {
                println!("export RESULT{}='{}'", num_results, serde_json::to_string(result).unwrap().replace("'", "\'"));
                num_results = num_results + 1;
            }
        }
        return true;
    } else {
        return false;
    }
}