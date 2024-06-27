const config = {
  content: ['./src/**/*.{html,js,svelte,ts}', './node_modules/flowbite-svelte/**/*.{html,js,svelte,ts}', './node_modules/flowbite-svelte-blocks/**/*.{html,js,svelte,ts}'],

  plugins: [require('flowbite/plugin'), require('flowbite-typography')],

  darkMode: 'class',

  theme: {
    extend: {
      colors: {
        light: {
          primary: '#7ec8e3',
          secondary: '#a3d9a5',
          background: '#ffffff',
          text: '#333333',
          accent: '#3b8eb5',
          green: '#68b36b',
          border: '#dddddd',
        },
        dark: {
          primary: '#0a3a5a',
          secondary: '#1b5e20',
          background: '#121212',
          text: '#e1e1e1',
          accent: '#1c7da7',
          green: '#4caf50',
          border: '#333333',
        },
        ocean: {
          primary: '#005f73',
          secondary: '#0a9396',
          background: '#e9d8a6',
          text: '#36454f',
          accent: '#94d2bd',
          green: '#99c1b9',
          border: '#eae2b7',
        },
      }
    }
  }
};

module.exports = config;