<script lang="ts">
    import { slide } from 'svelte/transition';
    let expanded = false;
</script>

<!-- svelte-ignore a11y-click-events-have-key-events -->
<div on:click|stopPropagation={() => (expanded = !expanded)} class="head" class:expanded>
    <slot name="head">Expand</slot>
</div>

{#if expanded}
<div transition:slide class="body">
    <slot />
</div>
{/if}

<style>
    .head:hover {
        cursor: pointer;
        user-select: none;
    }
    .head:before {
        content: '▸';
        float: left;
        margin-right: 5px;
    }
    .expanded:before {
        content: '▾';
    }
    .body {
        padding-left: 20px;
    }
</style>
