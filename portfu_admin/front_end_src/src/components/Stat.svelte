<script>
    import { onMount } from 'svelte';
    import { writable } from 'svelte/store';
  
    export let header = "Default Header";
    export let stat = "0";
    export let description = "Default Description";
    let statFontSize = writable('2rem'); // default font size
    let statRef;
  
    onMount(() => {
      adjustFontSize();
      window.addEventListener('resize', adjustFontSize);
    });
  
    function adjustFontSize() {
      const maxWidth = statRef.offsetWidth; // get the width of the element
      const newFontSize = Math.min(0.1 * maxWidth, 40); // calculate new font size
      statFontSize.set(`${newFontSize}px`);
    }
  </script>
  
  <div class="p-4 border border-gray-300 rounded-lg w-64">
    <h3 class="text-sm font-semibold">{header}</h3>
    <p class="font-bold my-2" bind:this={statRef} style="font-size: {$statFontSize};">{stat}</p>
    <p class="text-sm text-gray-500">{description}</p>
  </div>
  