<script>
    import { createEventDispatcher } from 'svelte';
    import { loggedInStore } from '$lib/stores.js';
    import logo from "$lib/assets/logo.webp";

    let username = '';
    let password = '';
    const dispatch = createEventDispatcher();

    function handleLogin() {
        // TODO auth logic
        if (username === 'user' && password === 'pass') {
            loggedInStore.set(true);
            dispatch('login');
        } else {
            alert('Invalid Credentials');
        }
    }
</script>

<style>
    login-container {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        height: 100vh;
        @apply bg-slate-950;
    }
    .login-form {
        display: flex;
        flex-direction: column;
        padding: 20px;

    }
    .login-form input {
        margin: 5px;
    }
</style>

<login-container>
    <img class="max-w-60" alt="Logo" src={logo} />

    <div class="my-8">
        <span class="text-xl text-white font-bold">portfu admin</span>
    </div>
    <form class="login-form" on:submit|preventDefault={handleLogin}>
        <input type="text" placeholder="Username" bind:value={username} />
        <input type="password" placeholder="Password" bind:value={password} />
        <button type="submit">Login</button>
    </form>
</login-container>
