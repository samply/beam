<script lang="ts">
    import { tasks } from "../store";
    import type { Task } from "../task";
    import TaskView from "./TaskView.svelte";


    // Filter ideas: from, to, hide successfully finished 
    let filters: Array<(update: Task) => boolean> = [];

</script>

<header>
    <h2>Tasks</h2>
</header>
<ul>
    {#each $tasks as task}
        {#if filters.every((filter) => filter(task))}
            <li>
                <TaskView {task} />
            </li>
        {:else}
            <pre>Not rendering Task from {task.task.from}</pre>
        {/if}
    {/each}
</ul>

<style>
    li {
        list-style: none;
    }
</style>
