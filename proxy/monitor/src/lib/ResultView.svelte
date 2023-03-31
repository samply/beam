<script lang="ts">
    import type { MsgTaskResult } from "../types";

    export let result: MsgTaskResult;
</script>

<div>
    <span>From: {result.from}</span>
    {#each result.to as to}
        <li>
            To: {to} 
        </li>
    {/each}
</div>
