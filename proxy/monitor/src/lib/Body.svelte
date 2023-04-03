<script lang="ts">
    import JSONTree from 'svelte-json-tree';
    import Expandable from './Expandable.svelte';
    export let json: any;
</script>

{#if typeof json === "string" && json.length > 100}
    <Expandable>
        <span slot="head">Large string body. Click to show.</span>
        <div class="body">
            Body: 
            <JSONTree value={json} />
        </div>
    </Expandable>
{:else}
 <div class="body">
    Body: 
    <JSONTree value={json} />
</div>
{/if}


<style>
.body {
    display: flex;
    align-items: flex-end;
}
</style>
