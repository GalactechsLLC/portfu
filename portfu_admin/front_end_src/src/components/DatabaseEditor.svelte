<script>
    import { databaseStore } from '$lib/stores.js';
    import { get } from 'svelte/store';

    let newRecord = { id: null, name: '', email: '' };
    let editingRecord = null;

    function addRecord() {
        if (newRecord.name && newRecord.email) {
            databaseStore.update(records => [
                ...records,
                { ...newRecord, id: records.length ? Math.max(...records.map(r => r.id)) + 1 : 1 }
            ]);
            newRecord = { id: null, name: '', email: '' };
        }
    }

    function editRecord(record) {
        editingRecord = { ...record };
    }

    function saveRecord() {
        databaseStore.update(records =>
            records.map(record => (record.id === editingRecord.id ? editingRecord : record))
        );
        editingRecord = null;
    }

    function deleteRecord(id) {
        databaseStore.update(records => records.filter(record => record.id !== id));
    }
</script>

<style>
    .database-editor {
        max-width: 600px;
        margin: 0 auto;
        text-align: center;
    }
    table {
        width: 100%;
        border-collapse: collapse;
        margin-bottom: 20px;
    }
    th, td {
        border: 1px solid #ddd;
        padding: 8px;
        text-align: left;
    }
    th {
        background-color: #333;
    }
    button {
        margin-right: 5px;
    }
</style>

<div class="database-editor">
    <h1>Database Editor</h1>

    <table>
        <thead>
        <tr>
            <th>ID</th>
            <th>Name</th>
            <th>Email</th>
            <th>Actions</th>
        </tr>
        </thead>
        <tbody>
        {#each $databaseStore as record}
            <tr>
                <td>{record.id}</td>
                <td>{record.name}</td>
                <td>{record.email}</td>
                <td>
                    <button on:click={() => editRecord(record)}>Edit</button>
                    <button on:click={() => deleteRecord(record.id)}>Delete</button>
                </td>
            </tr>
        {/each}
        </tbody>
    </table>

    {#if editingRecord}
        <div>
            <h2>Edit Record</h2>
            <div>
                <label>Name:</label>
                <input type="text" bind:value={editingRecord.name} />
            </div>
            <div>
                <label>Email:</label>
                <input type="email" bind:value={editingRecord.email} />
            </div>
            <button on:click={saveRecord}>Save</button>
            <button on:click={() => (editingRecord = null)}>Cancel</button>
        </div>
    {/if}

    <div>
        <h2>Add New Record</h2>
        <div>
            <label>Name:</label>
            <input type="text" bind:value={newRecord.name} />
        </div>
        <div>
            <label>Email:</label>
            <input type="email" bind:value={newRecord.email} />
        </div>
        <button on:click={addRecord}>Add</button>
    </div>
</div>
