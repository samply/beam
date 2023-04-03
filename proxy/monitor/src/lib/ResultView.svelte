<script lang="ts">
    import type { MsgTaskResult } from "../types";
    import JSONTree from 'svelte-json-tree';

    export let result: MsgTaskResult;
</script>

<div>
    <span>From: {result.from}</span>
    <ul>
        {#each result.to as to}
            <li>
                To: {to}
            </li>
        {/each}
    </ul>
    <JSONTree value={result.body} />
</div>
