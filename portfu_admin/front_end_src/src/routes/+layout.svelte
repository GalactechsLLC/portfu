<script>
    import "../app.pcss";
    import Header from '../components/Header.svelte';
    import Footer from '../components/Footer.svelte';
    import LeftNav from "../components/LeftNav.svelte";
    import {loggedInStore} from "$lib/stores.js";
    import Login from "../components/Login.svelte";
    import {Content} from "flowbite-svelte-blocks";
</script>


{#if $loggedInStore === true}
    <Header></Header>
    <Content class='w-full flex flex-grow'>
        <LeftNav></LeftNav>
        <div class="flex-grow">
            <slot />
        </div>
    </Content>
    <Footer></Footer>
{:else}
    <Login></Login>
{/if}
