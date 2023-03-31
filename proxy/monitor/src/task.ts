import type { MsgTaskRequest, MsgTaskResult } from "./types";

export class Task {
    task: MsgTaskRequest;
    results: MsgTaskResult[]
    
    constructor(task: MsgTaskRequest) {
        this.task = task;
        this.results = [];
    }
    
}
