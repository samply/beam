
<script lang="ts">
    let expanded = false;
</script>

<!-- svelte-ignore a11y-click-events-have-key-events -->
<div on:click|stopPropagation={() => expanded = !expanded} class="head">
    <slot name="head">
        Expand
    </slot>
</div>

<div  hidden={!expanded}>
    <slot />
</div>

<style>
    .head:hover {
        cursor: pointer;
    }
</style>
