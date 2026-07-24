const chartCache = new WeakMap();
let tooltip;
let tooltipChart;
let tooltipN;

function chartTooltip() {
  if (tooltip) return tooltip;

  tooltip = document.createElement("div");
  tooltip.className = "fft-chart-tooltip";
  tooltip.setAttribute("role", "tooltip");
  tooltip.hidden = true;
  document.body.append(tooltip);
  return tooltip;
}

function hideTooltip() {
  if (tooltip) tooltip.hidden = true;
}

function tooltipCell(className, text) {
  const cell = document.createElement("span");
  cell.className = className;
  cell.textContent = text;
  return cell;
}

function factorizationCell(factorization) {
  const cell = tooltipCell("fft-chart-tooltip-value", "");
  const pattern = /(\d+)(?:\^(\d+))?/g;
  let previousEnd = 0;
  let match;

  while ((match = pattern.exec(factorization)) !== null) {
    cell.append(factorization.slice(previousEnd, match.index), match[1]);
    if (match[2]) {
      const exponent = document.createElement("sup");
      exponent.textContent = match[2];
      cell.append(exponent);
    }
    previousEnd = pattern.lastIndex;
  }
  cell.append(factorization.slice(previousEnd));
  return cell;
}

function showTooltip(chart, n, clientX, clientY) {
  const element = chartTooltip();
  if (tooltipChart !== chart || tooltipN !== n) {
    const points = chart.querySelectorAll(
      `[data-chart-point][data-n="${n}"]`,
    );
    const fragment = new DocumentFragment();

    fragment.append(
      tooltipCell("fft-chart-tooltip-label", "N"),
      tooltipCell("fft-chart-tooltip-value", n),
      tooltipCell("fft-chart-tooltip-label", "Factors"),
      factorizationCell(points[0]?.dataset.factors ?? "—"),
    );

    const divider = document.createElement("span");
    divider.className = "fft-chart-tooltip-divider";
    fragment.append(divider);

    for (const point of points) {
      fragment.append(
        tooltipCell("fft-chart-tooltip-label", point.dataset.series),
        tooltipCell("fft-chart-tooltip-value", point.dataset.value),
      );
    }

    element.replaceChildren(fragment);
    tooltipChart = chart;
    tooltipN = n;
  }
  element.hidden = false;

  const gap = 12;
  const padding = 8;
  const bounds = element.getBoundingClientRect();
  let left = clientX + gap;
  let top = clientY - bounds.height / 2;

  if (left + bounds.width > window.innerWidth - padding) {
    left = clientX - gap - bounds.width;
  }
  top = Math.max(
    padding,
    Math.min(top, window.innerHeight - bounds.height - padding),
  );

  element.style.left = `${left}px`;
  element.style.top = `${top}px`;
}

function chartPoints(chart) {
  let points = chartCache.get(chart);
  if (points) return points;

  const seen = new Set();
  points = [];
  for (const point of chart.querySelectorAll("[data-chart-point]")) {
    const n = point.dataset.n;
    if (seen.has(n)) continue;
    seen.add(n);
    points.push({ n, x: Number(point.getAttribute("cx")) });
  }

  chartCache.set(chart, points);
  return points;
}

function clearChart(chart) {
  chart.removeAttribute("data-highlighted");
  for (const point of chart.querySelectorAll(
    "[data-chart-point].is-highlighted",
  )) {
    point.classList.remove("is-highlighted");
  }

  const guide = chart.querySelector("[data-chart-guide]");
  guide?.classList.remove("is-visible");
  hideTooltip();

  const root = chart.closest("main") ?? document;
  root
    .querySelector("[data-benchmark-row].is-highlighted")
    ?.classList.remove("is-highlighted");
}

function highlightNearest(chart, clientX, clientY) {
  const bounds = chart.getBoundingClientRect();
  const viewBox = chart.viewBox.baseVal;
  const x = viewBox.x + ((clientX - bounds.left) / bounds.width) * viewBox.width;
  const points = chartPoints(chart);

  let nearest = points[0];
  for (const point of points) {
    if (Math.abs(point.x - x) < Math.abs(nearest.x - x)) nearest = point;
  }
  if (!nearest) return;
  if (chart.dataset.highlighted === nearest.n) {
    showTooltip(chart, nearest.n, clientX, clientY);
    return;
  }

  clearChart(chart);
  chart.dataset.highlighted = nearest.n;

  for (const point of chart.querySelectorAll(
    `[data-chart-point][data-n="${nearest.n}"]`,
  )) {
    point.classList.add("is-highlighted");
  }

  const guide = chart.querySelector("[data-chart-guide]");
  if (guide) {
    guide.setAttribute("x1", nearest.x);
    guide.setAttribute("x2", nearest.x);
    guide.classList.add("is-visible");
  }
  showTooltip(chart, nearest.n, clientX, clientY);

  const root = chart.closest("main") ?? document;
  const row = root.querySelector(
    `[data-benchmark-row][data-n="${nearest.n}"]`,
  );
  if (!row) return;

  row.classList.add("is-highlighted");
  const scroller = row.closest("[data-benchmark-table-scroll]");
  if (scroller) {
    scroller.scrollTop =
      row.offsetTop - (scroller.clientHeight - row.offsetHeight) / 2;
  }
}

document.addEventListener("pointermove", (event) => {
  const chart = event.target.closest?.("[data-fft-chart]");
  if (chart) {
    highlightNearest(chart, event.clientX, event.clientY);
    return;
  }

  for (const activeChart of document.querySelectorAll(
    "[data-fft-chart][data-highlighted]",
  )) {
    clearChart(activeChart);
  }
});

document.addEventListener("pointerout", (event) => {
  const chart = event.target.closest?.("[data-fft-chart]");
  if (chart && (!event.relatedTarget || !chart.contains(event.relatedTarget))) {
    clearChart(chart);
  }
});

document.addEventListener(
  "pointerleave",
  (event) => {
    if (event.target.matches?.("[data-fft-chart]")) clearChart(event.target);
  },
  true,
);

document.addEventListener("click", (event) => {
  if (!event.target.closest?.('[role="group"][aria-label="Chart metric"]')) return;
  for (const chart of document.querySelectorAll("[data-fft-chart]")) {
    clearChart(chart);
  }
});
