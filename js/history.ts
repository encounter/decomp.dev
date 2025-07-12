import uPlot from 'uplot';

const height = 400;
const stroke = '#a9a9b3';
const grid = {
  stroke: 'rgba(128, 128, 128, 0.1)',
};

function percentValue(
  _self: uPlot,
  rawValue: number,
  _seriesIdx: number,
  _idx: number | null,
) {
  if (rawValue > 99.99 && rawValue < 100.0) {
    rawValue = 99.99;
  }
  return rawValue == null ? '' : `${rawValue.toFixed(2)}%`;
}

function renderChart(id: string, data: ReportHistoryEntry[]) {
  const chart = document.getElementById(id);
  if (!chart) {
    console.error(`Chart element with id ${id} not found`);
    return;
  }

  data.reverse();

  function getSize() {
    const container = chart!.parentElement;
    if (container) {
      return { width: container.offsetWidth, height };
    }
    return { width: 600, height };
  }

  const u = new uPlot(
    {
      ...getSize(),
      scales: {
        x: {
          time: true,
        },
      },
      series: [
        {},
        {
          show: false,
          label: 'Fuzzy Match Percent',
          width: 2,
          stroke: '#003f5c',
          value: percentValue,
        },
        {
          label: 'Matched Code',
          width: 2,
          stroke: '#ff6361',
          value: percentValue,
        },
        {
          show: false,
          label: 'Matched Data',
          width: 2,
          stroke: '#ffa600',
          value: percentValue,
        },
        {
          show: false,
          label: 'Linked Code',
          width: 2,
          stroke: '#bc5090',
          value: percentValue,
        },
        {
          show: false,
          label: 'Linked Data',
          width: 2,
          stroke: '#58508d',
          value: percentValue,
        },
      ],
      axes: [
        {
          stroke,
          grid,
        },
        {
          stroke,
          grid,
          values: (_self, ticks) =>
            ticks.map((rawValue) => `${rawValue.toFixed(0)}%`),
        },
      ],
      hooks: {
        init: [
          (u) => {
            u.over.addEventListener('click', (_e) => {
              const idx = u.legend.idx;
              if (idx != null) {
                console.log('click!', data[idx]);
              }
            });
          },
        ],
      },
    },
    [
      data.map((e) => Date.parse(e.timestamp) / 1000),
      data.map((e) => e.measures.fuzzy_match_percent || null),
      data.map((e) => e.measures.matched_code_percent || null),
      data.map((e) => e.measures.matched_data_percent || null),
      data.map((e) => e.measures.complete_code_percent || null),
      data.map((e) => e.measures.complete_data_percent || null),
    ],
    chart,
  );

  function updateSize() {
    u.setSize(getSize());
  }

  window.addEventListener('resize', updateSize);
}

window.renderChart = renderChart;
