<script lang="ts">
    import type { MsgTaskResult } from "../types";
    import Expandable from "./Expandable.svelte";
    import Body from "./Body.svelte";
    import JSONTree from 'svelte-json-tree';

    export let result: MsgTaskResult;

</script>

<div class="result">
    <Expandable>
        <span slot="head" class="status" title="{result.status}" style="background: var(--color-{result.status});">From: {result.from}</span>
        <div>
            Result for:
            <ul>
                {#each result.to as to}
                    <li>{to}</li>
                {/each}
            </ul>
        </div>

        <Body json={result.body} />
        <Expandable>
            <span slot="head">Show extra info</span>
            <div>
                Id: {result.task}
                <br />
                Status: {result.status}
                <br />
                Metadata:
                <JSONTree value={result.metadata} />
            </div>
        </Expandable>
    </Expandable>
</div>

<style>
    .status {
        border-radius: 2rem;
        padding: 0rem .5rem;
    }
</style>
