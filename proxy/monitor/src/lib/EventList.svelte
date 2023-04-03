<script lang="ts">
    import { derived, writable } from "svelte/store";
    import { tasks } from "../store";
    import type { Task } from "../task";
    import TaskView from "./TaskView.svelte";

    // Filter ideas: from, to, hide successfully finished 
    let from_filter_value = writable("");
    let from_filter = derived(from_filter_value, ($from_filter_value) => (task: Task) => {
        // If from_filter is not an empty string or undefined always return true otherwise check from field
        return !$from_filter_value || task.task.from === $from_filter_value
    });
    let filters = derived([from_filter], (filters) => filters, [$from_filter]);
    // Update filtered tasks whenever a new task or a new filter gets added
    let filtered_tasks = derived(
        [tasks, filters],
        ([$tasks, $filters]) => $tasks.filter(task => $filters.every(filter => filter(task))));
    $: console.log($filtered_tasks, $from_filter_value);
</script>

<header>
    <h2>Tasks</h2>
</header>
<div>
    <div class="settings">
        <input type="text" bind:value={$from_filter_value}>
        <button on:click={() => $tasks = []}>Clear Tasks</button>
    </div>
    <ul>
        {#each $filtered_tasks as task}
            <li>
                <TaskView {task} />
            </li>
        {/each}
    </ul>
</div>

<style>
    li {
        list-style: none;
    }
</style>
