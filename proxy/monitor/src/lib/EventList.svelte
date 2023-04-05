<script lang="ts">
    import { derived, writable } from "svelte/store";
    import { tasks } from "../store";
    import type { Task } from "../task";
    import TaskView from "./TaskView.svelte";

    // Filter ideas: from, to, hide successfully finished 
    let from_filter_value = writable("");
    let from_filter = derived(from_filter_value, ($from_filter_value) => (task: Task) => {
        // If from_filter is not an empty string or undefined always return true otherwise check from field
        return !$from_filter_value || task.task.from.includes($from_filter_value)
    });

    let done_filter = writable(false);
    let hide_done_filter = derived(done_filter, ($done_filter) => (task: Task) => {
        return !$done_filter || !Array.from(task.results.values()).every(result => result?.status === "succeeded")
    });
    
    let incoming_filter_active = writable(false);
    let hide_incoming_filter = derived(incoming_filter_active, ($incoming_filter_active) => (task: Task) => {
        return !$incoming_filter_active || !task.is_incoming
    });

    let filters = derived([from_filter, hide_done_filter, hide_incoming_filter], (filters) => filters, [$from_filter, $hide_done_filter, $hide_incoming_filter]);
    // Update filtered tasks whenever a new task or a new filter gets added
    let filtered_tasks = derived(
        [tasks, filters],
        ([$tasks, $filters]) => $tasks.filter(task => $filters.every(filter => filter(task))));
</script>

<header>
    <h2>Tasks</h2>
</header>
<div>
    <div class="settings">
        <button on:click={() => $tasks = []}>Clear Tasks</button>
        <button class:active={$done_filter} on:click={() => $done_filter = !$done_filter}>Hide completed</button>
        <button class:active={$incoming_filter_active} on:click={() => $incoming_filter_active = !$incoming_filter_active}>Hide incoming</button>
        <span>Filter from:</span>
        <input type="text" class="task-filter" bind:value={$from_filter_value}>
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
    button {
        background-color: var(--color-gray);
        outline: none;
    }
    .task-filter {
        border-radius: 1rem;
        font-size: inherit;
        padding: .2rem .4rem;
    }
    .settings {
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 0 3cqw;
        gap: 1rem;
    }
    .active {
        outline: solid #646cff 2px;
    }
</style>
