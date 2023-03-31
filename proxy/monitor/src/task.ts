import type { MsgTaskRequest, MsgTaskResult } from "./types";

export class Task {
    task: MsgTaskRequest;
    results: Map<string, MsgTaskResult | null>
    
    constructor(task: MsgTaskRequest) {
        this.task = task;
        this.results = new Map(task.to.map(t => [t, null]));
    }
    
    add_result(result: MsgTaskResult) {
        if (this.task.to.includes(result.from)) {
            this.results.set(result.from, result);
        } else {
            console.log("Result is not from this task");
        }
    }
    
}
