<script>
    let searchText = '';

    function handleSearch() {
        window.location.href = `/farmer/${searchText}`;
    }
</script>

<style>
    .search-container {
        display: flex;
        align-items: center;
        border: 1px solid grey;
        border-radius: 4px;
        padding: 5px;
        max-width: 400px;
    }

    .search-input {
        flex-grow: 1;
        border: none;
        outline: none;
        background-color: transparent;
        padding: 5px;
        font-size: 16px;
        color: white;
    }

    .search-button {
        background-color: green;
        border: none;
        border-radius: 4px;
        cursor: pointer;
        padding: 5px 10px;
        color: white;
        display: flex;
        align-items: center;
        justify-content: center;
    }

    .search-icon {
        display: inline-block;
        margin-right: 5px;
    }
</style>

<div class="search-container">
    <input 
        type="text" 
        class="search-input" 
        bind:value={searchText} 
        on:keydown={(e) => e.key === 'Enter' && handleSearch()}
        placeholder="Search by payout address"
    />
    <button class="search-button" on:click={handleSearch}>
        <span class="search-icon">
            <!-- Search Icon SVG -->
            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" fill="currentColor" viewBox="0 0 24 24" stroke="white">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.5 14.5L19 18m-6-6a7 7 0 117-7 7 7 0 01-7 7z"/>
            </svg>
        </span>
    </button>
</div>
