<script lang="ts">
    import JSONTree from 'svelte-json-tree';
    export let json: any;
</script>


<div class="body">
    Body: 
    <JSONTree value={json} />
</div>

<style>
.body {
    display: flex;
    align-items: flex-end;
}
</style>
