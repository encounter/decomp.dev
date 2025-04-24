const height = 400;
const stroke = '#a9a9b3';
const grid = {
    stroke: 'rgba(128, 128, 128, 0.1)',
};

function percentValue(self, rawValue) {
    if (rawValue == null) {
        return null;
    }
    return rawValue.toFixed(2) + "%";
}

type Measures = {
    fuzzy_match_percent: number;
    total_code: number;
    matched_code: number;
    matched_code_percent: number;
    total_data: number;
    matched_data: number;
    matched_data_percent: number;
    total_functions: number;
    matched_functions: number;
    matched_functions_percent: number;
    complete_code: number;
    complete_code_percent: number;
    complete_data: number;
    complete_data_percent: number;
    total_units: number;
    complete_units: number;
};

type ReportHistoryEntry = {
    timestamp: string;
    commit_sha: string;
    measures: Measures;
};

function renderChart(id: string, data: ReportHistoryEntry[]) {
    let chart = document.getElementById(id);
    if (!chart) {
        console.error(`Chart element with id ${id} not found`);
        return;
    }
    data.reverse();

    const u = new uPlot({
        id: id,
        width: 600,
        height: height,
        scales: {
            x: {
                time: true,
            },
        },
        series: [
            {},
            {
                show: false,
                label: "Fuzzy Match Percent",
                width: 2,
                stroke: "#003f5c",
                value: percentValue,
            },
            {
                label: "Matched Code",
                width: 2,
                stroke: "#ff6361",
                value: percentValue,
            },
            {
                show: false,
                label: "Matched Data",
                width: 2,
                stroke: "#ffa600",
                value: percentValue,
            },
            {
                show: false,
                label: "Linked Code",
                width: 2,
                stroke: "#bc5090",
                value: percentValue,
            },
            {
                show: false,
                label: "Linked Data",
                width: 2,
                stroke: "#58508d",
                value: percentValue,
            }
        ],
        axes: [
            {
                stroke,
                grid,
            },
            {
                stroke,
                grid,
                values: (self, ticks) => ticks.map(rawValue => rawValue.toFixed(0) + "%"),
            },
        ],
        hooks: {
            init: [
                u => {
                    u.over.addEventListener('click', e => {
                        console.log('click!', data[u.legend.idx]);
                    });
                }
            ],
        },
    }, null, chart);

    function updateSize() {
        const container = chart.parentElement;
        u.setSize({width: container.offsetWidth, height});
    }

    window.addEventListener('resize', updateSize);
    updateSize();
    u.setData([
        data.map(e => Date.parse(e.timestamp) / 1000),
        data.map(e => e.measures.fuzzy_match_percent || null),
        data.map(e => e.measures.matched_code_percent || null),
        data.map(e => e.measures.matched_data_percent || null),
        data.map(e => e.measures.complete_code_percent || null),
        data.map(e => e.measures.complete_data_percent || null),
    ]);
}
