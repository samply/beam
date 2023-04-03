<script lang="ts">
    import type { MsgTaskResult, WorkStatus } from "../types";
    import Expandable from "./Expandable.svelte";
    import Body from "./Body.svelte";

    export let result: MsgTaskResult;

</script>

<div class="result">
    <Expandable>
        <span slot="head" class="status" style="background: var(--color-{result.status});">From: {result.from}</span>
        <ul>
            {#each result.to as to}
                <li>
                    To: {to}
                </li>
            {/each}
        </ul>

        <Body json={result.body} />
    </Expandable>
</div>

<style>
    .status {
        border-radius: 2rem;
        padding: 0rem .5rem;
    }
</style>
