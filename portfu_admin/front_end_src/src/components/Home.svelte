<script>

    import Chart from "./Chart.svelte";

    let stats = [
        { title: "Total Visits", value: "9.8K" },
        { title: "Sales", value: "$24.5K" },
        { title: "New Users", value: "1.2K" }
    ];

    let chartOptions = {
        series: [{
            name: 'Sales',
            data: [31, 40, 28, 51, 42, 109, 100]
        }],
        chart: {
            height: 350,
            type: 'area'
        },
        dataLabels: {
            enabled: false
        },
        stroke: {
            curve: 'smooth'
        },
        xaxis: {
            type: 'datetime',
            categories: ["2021-03-18T00:00:00", "2021-03-19T01:30:00", "2021-03-19T02:30:00", "2021-03-19T03:30:00", "2021-03-19T04:30:00", "2021-03-19T05:30:00", "2021-03-19T06:30:00"]
        },
        tooltip: {
            x: {
                format: 'dd/MM/yy HH:mm'
            }
        }
    };
</script>

<div class="flex flex-wrap -m-4">
    {#each stats as stat}
        <div class="w-full md:w-1/3 p-4">
            <div class="bg-gray-800 p-6 rounded-lg shadow-md">
                <h2 class="text-xl font-semibold mb-2">{stat.title}</h2>
                <h3 class="text-3xl font-bold">{stat.value}</h3>
            </div>
        </div>
    {/each}
</div>

<div class="p-4">
    <Chart options="{chartOptions}" />
</div>

<style>
    /* Add any additional CSS here */
</style>