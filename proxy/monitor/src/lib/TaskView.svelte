<script lang="ts">
    import type { Task } from "../task";
    import ResultView from "./ResultView.svelte";

    export let task: Task;
    $: console.log(typeof task);
</script>

<div class="task">
    <div>From: {task.task.from}</div>
    <ul>
        {#each [...task.results] as [to, result]}
            <li>
                {#if result}
                <ResultView {result} />
                {:else}
                <span>To: {to}</span>
                {/if}
            </li>
        {/each}
    </ul>
</div>

<style>
.task {
    background-color: var(--color-gray);
    border-radius: 2rem;
    margin: 1rem;
    text-align: start;
}
</style>
