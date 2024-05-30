<script>
    import "../app.pcss";
    import Header from './../components/Header.svelte';
    import Footer from './../components/Footer.svelte';
    import LeftNav from "../components/LeftNav.svelte";
    import {loggedInStore} from "$lib/stores.js";
    import Login from "../components/Login.svelte";

</script>


{#if $loggedInStore === true}
    <Header></Header>
    <container class='w-full flex'>
        <LeftNav></LeftNav>
        <slot></slot>
    </container>
    <Footer></Footer>
{:else}
    <Login></Login>
{/if}
