import { writable } from "svelte/store";
import { Task } from "./task";
import type { MonitoringUpdate, MsgTaskRequest, MsgTaskResult } from "./types";

export const tasks = writable<Task[]>([])
        
function append_result(result: MsgTaskResult) {
    let task: Task;
    tasks.subscribe((ts => task = ts.find((t) => result.task === t.task.id)));
    if (task) {
        task.add_result(result);
    } else {
        console.log("Could not find task for result", result);
    }
}
        // TODO change this to actual host
const sse_stream = new EventSource("/monitor/events");
sse_stream.addEventListener("message", (e) => {
    // We cant push as we need svelte to understand that we updated this and need to rerender
    let update = JSON.parse(e.data) as MonitoringUpdate;
    if ("request" in update) {
        let msg = update.request.json;
        let request = update.request;
        console.log("request:", request);
        // We have a MsgTaskRequest from some App in the proxies network to the beam network
        if ("id" in msg) {
            tasks.update((ts) => [...ts, new Task(msg as MsgTaskRequest)]);
        // We have a MsgTaskResult from some App in the proxies network
        } else if ("status" in msg) {
            append_result(msg);
        } else {
            console.log("Ignoring:", request);
        }
    } else if ("response" in update) {
        let msg = update.response.json;
        let response = update.response;
        console.log("response:", response);

        if (Array.isArray(msg)) {
            msg.forEach(append_result);
        }
        if ("id" in msg) {
            tasks.update((ts) => [...ts, new Task(msg as MsgTaskRequest)]);
        } else if ("status" in msg) {
            append_result(msg);
        } else {
            console.log("Ignoring:", response);
        }
    } else {
        console.log("Unknown update", update);
    }
})
