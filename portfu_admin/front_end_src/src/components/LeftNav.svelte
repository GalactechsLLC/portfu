<script>
    import { contentTab } from '$lib/stores.js';

    let currentTab;

    contentTab.subscribe(value => {
        currentTab = value;
    });

    function setContent(abr) {
        contentTab.set(abr);
    }
</script>

<nav>
    <button on:click={() => setContent('HM')} class:selected={currentTab === 'HM'}>Home</button>
    <button on:click={() => setContent('FE')} class:selected={currentTab === 'FE'}>File Editor</button>
    <button on:click={() => setContent('FM')} class:selected={currentTab === 'FM'}>File Manager</button>
    <button on:click={() => setContent('DE')} class:selected={currentTab === 'DE'}>Database Editor</button>
    <button on:click={() => setContent('BE')} class:selected={currentTab === 'BE'}>Blogs</button>
    <button on:click={() => setContent('AC')} class:selected={currentTab === 'AC'}>Account</button>
    <button on:click={() => setContent('UM')} class:selected={currentTab === 'UM'}>User Management</button>
</nav>

<style>
    nav button {
        cursor: pointer;
        padding: 10px;
        border: 1px solid #555;
        border-radius: 10px;
        width: 100%;
        margin-bottom: 5px;
        background-color: #444; /* Default background color */
    }

    nav button.selected {
        @apply bg-slate-900;
        border-color: #000;
    }

    nav {
        @apply w-60 p-6;
    }
</style>
