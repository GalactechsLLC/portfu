<script>
    import { onMount } from "svelte";
    let users = [
        { id: 1, name: 'John Doe', email: 'john@example.com', role: 'Admin' },
        { id: 2, name: 'Jane Smith', email: 'jane@example.com', role: 'User' },
    ];

    let editingId = null;
    let editFormData = {
        name: '',
        email: '',
        role: '',
    };

    function handleEditClick(user) {
        editingId = user.id;
        editFormData = { ...user };
    }

    function handleInputChange(event) {
        const { name, value } = event.target;
        editFormData = { ...editFormData, [name]: value };
    }

    function handleSaveClick(userId) {
        users = users.map(user => user.id === userId ? { ...user, ...editFormData } : user);
        editingId = null;
    }
</script>

<style>
    table {
        width: 100%;
        border-collapse: collapse;
    }

    th, td {
        border: 1px solid #ddd;
        padding: 8px;
    }

    th {
        background-color: #333;
    }

    button {
        padding: 5px 10px;
        margin: 0 5px;
    }

    input {
        width: 100%;
        box-sizing: border-box;
    }
</style>

<h1>User Manager</h1>
<table>
    <thead>
    <tr>
        <th>ID</th>
        <th>Name</th>
        <th>Email</th>
        <th>Role</th>
        <th>Actions</th>
    </tr>
    </thead>
    <tbody>
    {#each users as user}
        <tr>
            <td>{user.id}</td>
            <td>
                {#if editingId === user.id}
                    <input type="text" name="name" bind:value={editFormData.name} on:input={handleInputChange} />
                {:else}
                    {user.name}
                {/if}
            </td>
            <td>
                {#if editingId === user.id}
                    <input type="email" name="email" bind:value={editFormData.email} on:input={handleInputChange} />
                {:else}
                    {user.email}
                {/if}
            </td>
            <td>
                {#if editingId === user.id}
                    <input type="text" name="role" bind:value={editFormData.role} on:input={handleInputChange} />
                {:else}
                    {user.role}
                {/if}
            </td>
            <td>
                {#if editingId === user.id}
                    <button on:click={() => handleSaveClick(user.id)}>Save</button>
                {:else}
                    <button on:click={() => handleEditClick(user)}>Edit</button>
                {/if}
            </td>
        </tr>
    {/each}
    </tbody>
</table>