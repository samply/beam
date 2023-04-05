import type { MsgTaskRequest, MsgTaskResult } from "./types";

export class Task {
    task: MsgTaskRequest;
    /** if true the request is coming from the broker if false the request is coming from an app in the proxies network */
    is_incoming: boolean;
    results: Map<string, MsgTaskResult | null>
    
    constructor(task: MsgTaskRequest, is_incoming: boolean) {
        this.task = task;
        this.is_incoming = is_incoming;
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
