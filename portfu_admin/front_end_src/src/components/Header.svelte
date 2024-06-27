<script>
  import { Navbar, NavBrand, Button, Input, Dropdown, DropdownItem } from 'flowbite-svelte';
  import { SearchOutline } from 'flowbite-svelte-icons';
  import logo from '$lib/assets/logo.webp';

  function handleSearch(event) {
    // Check if the key pressed is 'Enter'
    if (event.key === 'Enter') {
      // Get the value from the input
      const searchTerm = event.target.value;
      // Navigate to /farmer/{input}
      window.location.href = `/${searchTerm}`;
    }
  }

  if (localStorage.theme === 'dark' || (!('theme' in localStorage) && window.matchMedia('(prefers-color-scheme: dark)').matches)) {
    document.documentElement.classList.add('dark')
  } else {
    document.documentElement.classList.remove('dark')
  }
  function resetTheme() {
    document.documentElement.classList.remove('system')
    document.documentElement.classList.remove('light')
    document.documentElement.classList.remove('dark')
  }

  function setLight() {
    resetTheme();
    localStorage.theme = 'light'
    document.documentElement.classList.add('light')
  }
  function setDark() {
    resetTheme();
    localStorage.theme = 'dark'
    document.documentElement.classList.add('dark')
  }

</script>

<Navbar class="dark:bg-slate-900">
  <NavBrand href="/">
    <img class="max-w-16" alt="Logo" src={logo} />
    <div class="ml-8">
      <span class="text-xl text-white font-bold">portfu admin</span>
    </div>
  </NavBrand>
  <div class="flex md:order-2">
    <theme_selector class="mr-4 relative self-center">
      <Button class="material-symbols-outlined text-md">planner_banner_ad_pt</Button>
      <Dropdown class='min-w-28'>
        <DropdownItem on:click={setLight}>Light</DropdownItem>
        <DropdownItem on:click={setDark}>Dark</DropdownItem>
      </Dropdown>
    </theme_selector>
    <Button color="none" data-collapse-toggle="mobile-menu-3" aria-controls="mobile-menu-3" aria-expanded="false" class="md:hidden text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 focus:outline-none focus:ring-4 focus:ring-gray-200 dark:focus:ring-gray-700 rounded-lg text-sm p-2.5 me-1">
      <SearchOutline class="w-5 h-5" />
    </Button>
    <div class="hidden relative md:block">
      <div class="flex absolute inset-y-0 start-0 items-center ps-3 pointer-events-none">
        <SearchOutline class="w-4 h-4" />
      </div>
      <Input id="search-navbar" class="ps-10" placeholder="Search" on:keydown={handleSearch} />
    </div>
  </div>
</Navbar>

<style>
  NavBar {
    @apply bg-slate-950;
  }
</style>
