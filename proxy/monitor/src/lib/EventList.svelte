<script lang="ts">
    import { onMount } from "svelte";
    import { Task } from "../task";
    import type { MonitoringUpdate, MsgTaskResult } from "../types";


    let tasks: Array<Task> = [];
    // Filter ideas: from, to, hide successfully finished 
    let filters: Array<(update: Task) => boolean> = [];

    function append_result(result: MsgTaskResult) {
        let task = tasks.find((t) => result.task === t.task.id);
        if (task !== undefined) {
            task.results.push(result);
        } else {
            console.log("Could not find task for result", result);
        }
    }

    onMount(() => {
        // TODO change this to actual host
        let sse_stream = new EventSource("/monitor/events");
        sse_stream.addEventListener("message", (e) => {
            // We cant push as we need svelte to understand that we updated this and need to rerender
            let update = JSON.parse(e.data) as MonitoringUpdate
            if ("request" in update) {
                let msg = update.request.json;
                let request = update.request;
                // We have a MsgTaskRequest from some App in the proxies network to the beam network
                if ("id" in msg) {
                    tasks = [...tasks, new Task(msg)];
                // We have a MsgTaskResult from some App in the proxies network
                } else if ("status" in msg) {
                    append_result(msg);
                } else {
                    console.log("Ignoring:", request);
                }
            } else if ("response" in update) {
                let msg = update.response.json;
                let response = update.response;

                if (Array.isArray(msg)) {
                    msg.forEach(append_result);
                }
                if ("id" in msg) {
                    tasks = [...tasks, new Task(msg)];
                } else if ("status" in msg) {
                    append_result(msg);
                } else {
                    console.log("Ignoring:", response);
                }
            } else {
                console.log("Unknown update", update);
            }
            tasks = [...tasks];
        })
    })
</script>

<header>
    <h2>Request Traffic</h2>
</header>
<ul>
    {#each tasks as request}
        {#if filters.every((filter) => filter(request))}
            <li>{request}</li>
        {/if}
    {/each}
</ul>

<style>
    li {
        list-style: none;
    }
</style>
