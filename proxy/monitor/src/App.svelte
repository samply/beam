<script lang="ts">
    import RequestList from "./lib/RequestList.svelte";
    import Nav from "./lib/Nav.svelte";
</script>

<main>
    <aside>
        <Nav />
    </aside>

    <div class="content">
        <RequestList />
    </div>
</main>

<style>
    main {
        display: grid;
        grid-template-columns: 1fr 4fr;
        height: 100%;
    }
    .content {
        padding: 1.5rem;
    }
    aside {
        background-color: whitesmoke;
        height: 100%;
    }
</style>
