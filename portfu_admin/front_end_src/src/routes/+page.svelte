<script>
    import { contentTab } from '$lib/stores.js';
    import FileEditor from "../components/FileEditor.svelte";
    import FileManager from "../components/FileManager.svelte";
    import Profile from "../components/Profile.svelte";
    import BlogEditor from "../components/BlogEditor.svelte";
    import DatabaseEditor from "../components/DatabaseEditor.svelte";
    import Home from "../components/Home.svelte";
    import UserManager from "../components/UserManager.svelte";

    contentTab.set('HM');
</script>

<content>
    {#if $contentTab === 'HM'}
        <Home />
    {:else if $contentTab === 'FE'}
        <FileEditor />
    {:else if $contentTab === 'FM'}
        <FileManager />
    {:else if $contentTab === 'DE'}
        <DatabaseEditor />
    {:else if $contentTab === 'BE'}
        <BlogEditor />
    {:else if $contentTab === 'AC'}
        <Profile />
    {:else if $contentTab === 'UM'}
        <UserManager />
    {:else}
        <div>Select a tab to display content.</div>
    {/if}
</content>

<style>
    content {
        display: flex;
        min-height: 88.5vh;
        padding: 30px 10px;
    }
</style>
