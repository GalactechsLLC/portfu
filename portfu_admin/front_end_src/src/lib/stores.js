import { writable } from 'svelte/store';

export const contentTab = writable('HM');

export const filesStore = writable([]);

export const databaseStore = writable([
    { id: 1, name: 'John Doe', email: 'john@example.com' },
    { id: 2, name: 'Jane Smith', email: 'jane@example.com' }
]);

export const postsStore = writable([
    { id: 1, title: 'First Post', content: 'This is the content of the first post.' },
    { id: 2, title: 'Second Post', content: 'This is the content of the second post.' }
]);

export const loggedInStore = writable(false);
