<!--<script>-->
<!--    let fileContent = '';-->
<!--    let fileName = '';-->

<!--    // Function to handle file input change-->
<!--    function handleFileChange(event) {-->
<!--        const file = event.target.files[0];-->
<!--        if (file) {-->
<!--            fileName = file.name;-->
<!--            const reader = new FileReader();-->
<!--            reader.onload = (e) => {-->
<!--                fileContent = e.target.result;-->
<!--            };-->
<!--            reader.readAsText(file);-->
<!--        }-->
<!--    }-->

<!--    // Function to save the edited content-->
<!--    function saveFile() {-->
<!--        const blob = new Blob([fileContent], { type: 'text/plain' });-->
<!--        const link = document.createElement('a');-->
<!--        link.href = URL.createObjectURL(blob);-->
<!--        link.download = fileName || 'untitled.txt';-->
<!--        link.click();-->
<!--    }-->
<!--</script>-->

<!--<style>-->
<!--    .file-editor {-->
<!--        display: flex;-->
<!--        flex-direction: column;-->
<!--        align-items: center;-->
<!--    }-->
<!--    textarea {-->
<!--        width: 100%;-->
<!--        height: 300px;-->
<!--        margin-top: 10px;-->
<!--    }-->
<!--    .actions {-->
<!--        margin-top: 10px;-->
<!--    }-->
<!--</style>-->

<!--<div class="file-editor">-->
<!--    <input type="file" accept=".txt" on:change={handleFileChange} />-->
<!--    <textarea bind:value={fileContent}></textarea>-->
<!--    <div class="actions">-->
<!--        <button on:click={saveFile}>Save</button>-->
<!--    </div>-->
<!--</div>-->

<!--<script>-->
<!--    import { AceEditor } from "svelte-ace";-->
<!--    let text = "";-->
<!--    let edit_lang = "js";-->

<!--    let theme = "gruvbox"-->

<!--    let readonly = false;-->
<!--    let visible = true;-->
<!--</script>-->

<!--<AceEditor-->
<!--        on:selectionChange={(obj) => console.log(obj.detail)}-->
<!--        on:paste={(obj) => console.log(obj.detail)}-->
<!--        on:input={(obj) => {-->
<!--            text = obj.detail;-->
<!--            console.log(obj.detail);-->
<!--          }}-->
<!--        on:focus={() => console.log("focus")}-->
<!--        on:documentChange={(obj) => console.log(`document change : ${obj.detail}`)}-->
<!--        on:cut={() => console.log("cut")}-->
<!--        on:cursorChange={() => console.log("cursor change")}-->
<!--        on:copy={() => console.log("copy")}-->
<!--        on:init={(editor) => console.log(editor.detail)}-->
<!--        on:commandKey={(obj) => console.log(obj.detail)}-->
<!--        on:changeMode={(obj) => console.log(`change mode : ${obj.detail}`)}-->
<!--        on:blur={() => console.log("blur")}-->
<!--        width="100%"-->
<!--        height="500px"-->
<!--        lang={edit_lang}-->
<!--        theme="gruvbox"-->
<!--        value={text}-->
<!--        {readonly}-->
<!--/>-->

<!--<button-->
<!--        style="opacity:{readonly ? 1 : 0.5}"-->
<!--        on:click={(e) => (readonly = !readonly)}>Toggle Readonly</button-->
<!--&gt;-->

<script lang="ts">
    import { onMount } from "svelte";
    import { AceEditor } from "svelte-ace";
    import { writable } from "svelte/store";

    interface Tab {
        id: string;
        text: string;
        mode: string;
        theme: string;
        readonly: boolean;
    }

    let text = "";
    let mode = "ace/mode/javascript";
    let theme = "gruvbox";
    let readonly = false;
    let tabCounter = 0;

    const tabs = writable<Tab[]>([]);
    let currentTab: number = 0;

    function addTab() {
        tabCounter++;
        const newTab: Tab = { id: `tab_${tabCounter}`, text: text, mode: mode, theme: theme, readonly: readonly };
        tabs.update(allTabs => [...allTabs, newTab]);
        currentTab = tabCounter - 1; // Set the current tab to the newly added tab
    }

    function removeTab(id: string) {
        tabs.update(allTabs => allTabs.filter(tab => tab.id !== id));
        if (currentTab >= tabs.length) {
            currentTab = tabs.length - 1; // Adjust currentTab if necessary
        }
    }

    function activateTab(index: number) {
        currentTab = index;
    }
</script>

<style>
    .tabs {
        display: flex;
        flex-direction: column;
    }

    .tab-list {
        display: flex;
        list-style: none;
        padding: 0;
    }

    .tab-list li {
        margin: 0 5px;
        cursor: pointer;
    }

    .tab-panel {
        border: 1px solid #ccc;
        padding: 10px;
        display: none;
    }

    .tab-panel.active {
        display: block;
    }

    .close {
        margin-left: 10px;
        cursor: pointer;
    }
</style>

<div class="tabs">
    <ul class="tab-list">
        {#each $tabs as tab, index (tab.id)}
            <button on:click={() => activateTab(index)}>
                {tab.id}
                <button class="close" on:click={(e) => { e.stopPropagation(); removeTab(tab.id); }}>✖</button>
            </button>
        {/each}
    </ul>

    {#each $tabs as tab, index (tab.id)}
        <div class="tab-panel {currentTab === index ? 'active' : ''}" id={tab.id}>
            <AceEditor
                    on:selectionChange={(obj) => console.log(obj.detail)}
                    on:paste={(obj) => console.log(obj.detail)}
                    on:input={(obj) => {
            tab.text = obj.detail;
            console.log(obj.detail);
          }}
                    on:focus={() => console.log("focus")}
                    on:documentChange={(obj) => console.log(`document change : ${obj.detail}`)}
                    on:cut={() => console.log("cut")}
                    on:cursorChange={() => console.log("cursor change")}
                    on:copy={() => console.log("copy")}
                    on:init={(editor) => console.log(editor.detail)}
                    on:commandKey={(obj) => console.log(obj.detail)}
                    on:changeMode={(obj) => console.log(`change mode : ${obj.detail}`)}
                    on:blur={() => console.log("blur")}
                    width="100%"
                    height="500px"
                    lang={tab.edit_lang}
                    theme={tab.theme}
                    value={tab.text}
                    {readonly}
            />
        </div>
    {/each}
</div>

<button on:click={addTab}>Add Tab</button>
<button style="opacity:{readonly ? 1 : 0.5}" on:click={() => (readonly = !readonly)}>Toggle Readonly</button>
