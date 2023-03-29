
<script>
    import beamLogo from '../assets/Logo.svg'
</script>

<nav>
    <a href="/" target="_blank" rel="noreferrer">
      <img src={beamLogo} class="logo" alt="Vite Logo" />
    </a>
</nav>


<style>

nav {
    padding: .5rem;
    display: flex;
    justify-content: center;
}

.logo {
    height: 4em;
    padding: .5em;
    will-change: filter;
    transition: filter 300ms;
}
    
</style>
