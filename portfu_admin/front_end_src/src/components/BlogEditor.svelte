<script>
    import { postsStore } from '$lib/stores.js';
    import { get } from 'svelte/store';

    let newPost = { id: null, title: '', content: '' };
    let editingPost = null;

    function addPost() {
        if (newPost.title && newPost.content) {
            postsStore.update(posts => [
                ...posts,
                { ...newPost, id: posts.length ? Math.max(...posts.map(p => p.id)) + 1 : 1 }
            ]);
            newPost = { id: null, title: '', content: '' };
        }
    }

    function editPost(post) {
        editingPost = { ...post };
    }

    function savePost() {
        postsStore.update(posts =>
            posts.map(post => (post.id === editingPost.id ? editingPost : post))
        );
        editingPost = null;
    }

    function deletePost(id) {
        postsStore.update(posts => posts.filter(post => post.id !== id));
    }
</script>

<style>
    .blog-editor {
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

<div class="blog-editor">
    <h1>Blog Editor</h1>

    <table>
        <thead>
        <tr>
            <th>ID</th>
            <th>Title</th>
            <th>Content</th>
            <th>Actions</th>
        </tr>
        </thead>
        <tbody>
        {#each $postsStore as post}
            <tr>
                <td>{post.id}</td>
                <td>{post.title}</td>
                <td>{post.content}</td>
                <td>
                    <button on:click={() => editPost(post)}>Edit</button>
                    <button on:click={() => deletePost(post.id)}>Delete</button>
                </td>
            </tr>
        {/each}
        </tbody>
    </table>

    {#if editingPost}
        <div>
            <h2>Edit Post</h2>
            <div>
                <label>Title:</label>
                <input type="text" bind:value={editingPost.title} />
            </div>
            <div>
                <label>Content:</label>
                <textarea bind:value={editingPost.content}></textarea>
            </div>
            <button on:click={savePost}>Save</button>
            <button on:click={() => (editingPost = null)}>Cancel</button>
        </div>
    {/if}

    <div>
        <h2>Add New Post</h2>
        <div>
            <label>Title:</label>
            <input type="text" bind:value={newPost.title} />
        </div>
        <div>
            <label>Content:</label>
            <textarea bind:value={newPost.content}></textarea>
        </div>
        <button on:click={addPost}>Add</button>
    </div>
</div>
