<script lang="ts">
    let requests = [{"test": "test"}];
</script>

<header>
    <h2>Request Traffic</h2>
</header>
<ul>
    {#each requests as request}
        <li>{request}</li>
    {/each}
</ul>
