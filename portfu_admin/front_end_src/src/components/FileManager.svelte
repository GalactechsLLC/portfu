<script>
    import { filesStore } from '$lib/stores.js';
    import { createEventDispatcher } from 'svelte';
    import FileEditor from './FileEditor.svelte';
    import { get } from 'svelte/store';

    const dispatch = createEventDispatcher();

    let selectedFile = null;
    let showFileEditor = false;

    function handleFileChange(event) {
        const file = event.target.files[0];
        if (file) {
            filesStore.update(files => [...files, file]);
            selectedFile = file;
            showFileEditor = true;
        }
    }

    function openFileEditor(file) {
        selectedFile = file;
        showFileEditor = true;
    }

    function isImage(file) {
        return file.type.startsWith('image/');
    }
</script>

<style>
    .file-manager {
        max-width: 600px;
        margin: 0 auto;
        text-align: center;
    }
    .file-list {
        list-style-type: none;
        padding: 0;
    }
    .file-list li {
        margin: 5px 0;
        cursor: pointer;
        color: blue;
        text-decoration: underline;
    }
    .file-preview img {
        max-width: 100%;
        height: auto;
        border-radius: 5px;
        margin: 10px 0;
    }
    input[type="file"] {
        display: none;
    }
    label {
        cursor: pointer;
        color: blue;
        text-decoration: underline;
    }
</style>

<div class="file-manager">
    <h1>File Manager</h1>
    <ul class="file-list">
        {#each $filesStore as file (file.name)}
            <li on:click={() => openFileEditor(file)}>{file.name}</li>
        {/each}
    </ul>
    <label for="fileInput">Upload File</label>
    <input
            type="file"
            id="fileInput"
            accept="*"
            on:change={handleFileChange}
    />

    {#if showFileEditor && selectedFile}
        <FileEditor {selectedFile} />
        {#if isImage(selectedFile)}
            <div class="file-preview">
                <img src={URL.createObjectURL(selectedFile)} alt={selectedFile.name} />
            </div>
        {/if}
    {/if}
</div>
