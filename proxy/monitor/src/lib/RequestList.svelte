<script lang="ts">
    import { onMount } from "svelte";
    import type { MonitoringUpdate } from "../types";


    let requests: Array<MonitoringUpdate> = [];

    onMount(() => {
        // TODO change this to actual host
        let sse_stream = new EventSource("http://localhost:8082/monitor/events");
        sse_stream.addEventListener("message", (e) => {
            // We cant push as we need svelte to understand that we updated this and need to rerender
            let update = JSON.parse(e.data) as MonitoringUpdate
            requests = [...requests, update];
        })
    })
</script>

<header>
    <h2>Request Traffic</h2>
</header>
<ul>
    {#each requests as request}
        <li>{request}</li>
    {/each}
</ul>
