const unitBounds = (unit: Unit, width: number, height: number) => {
  return {
    x: unit.x * width,
    y: unit.y * height,
    w: unit.w * width,
    h: unit.h * height,
  };
};

const BORDER_RADIUS = 5;
const PADDING_W = 10;
const PADDING_H = 5;
const MARGIN = 5;

const ellipsize = (
  ctx: CanvasRenderingContext2D,
  text: string,
  width: number,
) => {
  const ellipsis = '…';
  const padding = PADDING_W * 2;
  let m = ctx.measureText(text);
  if (m.actualBoundingBoxRight + m.actualBoundingBoxLeft + padding <= width) {
    return text;
  }
  let n = 3;
  while (true) {
    const start = text.length / 4 - n / 4;
    const ellipsized = text.slice(0, start) + ellipsis + text.slice(start + n);
    m = ctx.measureText(ellipsized);
    if (m.actualBoundingBoxRight + m.actualBoundingBoxLeft + padding <= width) {
      return ellipsized;
    }
    n++;
  }
};

const UNITS = ['B', 'kB', 'MB', 'GB', 'TB', 'PB', 'EB', 'ZB', 'YB'];

const formatSize = (value: number): string => {
  let unit = 0;
  while (value >= 1000.0 && unit < UNITS.length - 1) {
    // biome-ignore lint/style/noParameterAssign: don't care
    value /= 1000.0;
    unit += 1;
  }
  return `${value.toFixed(2)} ${UNITS[unit]}`;
};

const drawTooltip = (
  ctx: CanvasRenderingContext2D,
  unit: Unit,
  width: number,
  height: number,
) => {
  const style = getComputedStyle(ctx.canvas);
  const fontWeight = style.getPropertyValue('--font-weight') || 'normal';
  const fontSize = style.getPropertyValue('--font-size') || '16px';
  const fontFamily = style.getPropertyValue('--font-family') || 'sans-serif';
  const tooltipBackground =
    style.getPropertyValue('--tooltip-background') || '#fff';
  const tooltipColor = style.getPropertyValue('--tooltip-color') || '#000';
  ctx.font = `${fontWeight} ${fontSize} ${fontFamily}`;
  ctx.textBaseline = 'middle';

  const { x, y, w, h } = unitBounds(unit, width, height);
  let percent = unit.fuzzy_match_percent;
  if (percent > 99.99 && percent < 100.0) {
    percent = 99.99;
  }
  const text = ellipsize(
    ctx,
    `${unit.name} • ${formatSize(unit.total_code)} • ${percent.toFixed(2)}%`,
    width,
  );
  const m = ctx.measureText(text);
  const bw = m.actualBoundingBoxRight + m.actualBoundingBoxLeft + PADDING_W * 2;
  const bh = m.fontBoundingBoxAscent + m.fontBoundingBoxDescent + PADDING_H * 2;
  let bx = x + (w - bw) / 2;
  let by = y - bh - MARGIN;
  let ay = y;
  if (isTouch) {
    bx = (width - bw) / 2;
    if (y + h / 2 < height / 2) {
      // Draw at the bottom
      by = height - bh - MARGIN;
    } else {
      // Draw at the top
      by = MARGIN;
    }
  } else {
    if (bx + bw > width) {
      bx = width - bw;
    }
    if (bx < 0) {
      bx = 0;
    }
    if (by < 0) {
      // Draw below the box
      by = y + h + MARGIN;
      ay = y + h;
    }
    if (by + bh > height) {
      // Draw inside the box
      by = y + MARGIN;
      ay = y;
    }
  }
  ctx.fillStyle = tooltipBackground;
  ctx.beginPath();
  ctx.roundRect(bx, by, bw, bh, BORDER_RADIUS);
  if (!isTouch) {
    // Arrow
    const ax = x + w / 2;
    if (ay < by) {
      // Top
      ctx.moveTo(ax, ay);
      ctx.lineTo(ax + MARGIN, by);
      ctx.lineTo(ax - MARGIN, by);
    } else {
      // Bottom
      ctx.moveTo(ax, ay);
      ctx.lineTo(ax + MARGIN, by + bh);
      ctx.lineTo(ax - MARGIN, by + bh);
    }
  }
  ctx.fill();
  ctx.fillStyle = tooltipColor;
  ctx.fillText(text, bx + PADDING_W, by + bh / 2);
};

let hovered: Unit | null = null;
let dirty = false;
let isTouch = false;
let cachedCanvas: HTMLCanvasElement | null = null;
let unitsDirty = false;

const setup = (
  ctx: CanvasRenderingContext2D,
  ratio: number,
  width: number,
  height: number,
) => {
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0); // Scale to device pixel ratio
  ctx.clearRect(0, 0, width, height);
  // Clear the canvas with dark mode's background color, even in light mode.
  // This is so that transparency doesn't make the canvas look bad in light mode.
  ctx.fillStyle = '#181c25';
  ctx.fillRect(0, 0, width, height);
  ctx.lineWidth = 1;
  ctx.strokeStyle = '#000';
};

const drawUnits = (
  ctx: CanvasRenderingContext2D,
  units: Unit[],
  width: number,
  height: number,
) => {
  for (const unit of units) {
    const { x, y, w, h } = unitBounds(unit, width, height);

    let innerColor: string;
    let outerColor: string;
    if (unit.fuzzy_match_percent === 100.0) {
      innerColor = 'hsl(120 100% 39%)';
      outerColor = 'hsl(120 100% 17%)';
    } else {
      innerColor = `color-mix(in srgb, hsl(221 0% 21%), hsl(221 50% 35%) ${unit.fuzzy_match_percent}%)`;
      outerColor = `color-mix(in srgb, hsl(221 0% 5%), hsl(221 50% 15%) ${unit.fuzzy_match_percent}%)`;
    }
    const cx = x + w * 0.4;
    const cy = y + h * 0.4;
    const r0 = (w + h) * 0.1;
    const r1 = (w + h) * 0.5;
    const gradient = ctx.createRadialGradient(cx, cy, r0, cx, cy, r1);
    gradient.addColorStop(0, innerColor);
    gradient.addColorStop(1, outerColor);
    ctx.fillStyle = gradient;

    ctx.beginPath();
    ctx.rect(x, y, w, h);

    ctx.save();
    if (unit.filtered) {
      ctx.clip();
    }
    ctx.stroke();
    ctx.restore();

    if (unit.filtered) {
      ctx.globalAlpha = 0.1;
    }
    ctx.fill();
    ctx.globalAlpha = 1.0;
  }
};

const draw = (canvas: HTMLCanvasElement, units: Unit[]) => {
  const { width, height } = canvas.getBoundingClientRect();
  const ratio = window.devicePixelRatio;
  const renderWidth = Math.round(width * ratio);
  const renderHeight = Math.round(height * ratio);
  if (
    !dirty &&
    !unitsDirty &&
    canvas.width === renderWidth &&
    canvas.height === renderHeight
  ) {
    // Nothing changed
    return;
  }
  dirty = false;
  // High DPI support
  if (canvas.width !== renderWidth || canvas.height !== renderHeight) {
    canvas.width = renderWidth;
    canvas.height = renderHeight;
  }
  // Update cached canvas if needed
  if (!cachedCanvas) {
    cachedCanvas = document.createElement('canvas');
  }

  if (
    unitsDirty ||
    cachedCanvas.width !== renderWidth ||
    cachedCanvas.height !== renderHeight
  ) {
    unitsDirty = false;
    cachedCanvas.width = renderWidth;
    cachedCanvas.height = renderHeight;
    const cachedCtx = cachedCanvas.getContext('2d');
    if (!cachedCtx) {
      return;
    }
    setup(cachedCtx, ratio, width, height);
    drawUnits(cachedCtx, units, width, height);
  }

  const ctx = canvas.getContext('2d');
  if (!ctx) {
    return;
  }
  // Use 1:1 scale for rendering cached canvas
  setup(ctx, 1, renderWidth, renderHeight);
  ctx.drawImage(cachedCanvas, 0, 0);
  ctx.scale(ratio, ratio); // Restore device scale
  if (hovered) {
    const { x, y, w, h } = unitBounds(hovered, width, height);
    ctx.lineWidth = 2;
    ctx.strokeStyle = '#fff';
    ctx.strokeRect(x, y, w, h);
    drawTooltip(ctx, hovered, width, height);
  }
};

const findUnit = (
  canvas: HTMLCanvasElement,
  units: Unit[],
  clientX: number,
  clientY: number,
): Unit | null => {
  const { width, height, left, top } = canvas.getBoundingClientRect();
  const mx = clientX - left;
  const my = clientY - top;
  let nearOverlapUnit = null;
  const epsilon = 3;
  for (const unit of units) {
    if (unit.filtered) {
      continue;
    }
    const { x, y, w, h } = unitBounds(unit, width, height);
    if (mx >= x && mx <= x + w && my >= y && my <= y + h) {
      return unit;
    }
    // If the unit doesn't exactly overlap the cursor, check if it's within a few pixels of overlapping.
    // This is needed to make it possible to hover and click units that have subpixel widths/heights.
    if (
      !nearOverlapUnit &&
      mx >= x - epsilon &&
      mx <= x + w + epsilon &&
      my >= y - epsilon &&
      my <= y + h + epsilon
    ) {
      nearOverlapUnit = unit;
    }
  }
  if (nearOverlapUnit) {
    return nearOverlapUnit;
  }
  return null;
};

const drawTreemap = (id: string, clickable: boolean, units: Unit[]) => {
  const canvas = document.getElementById(id) as HTMLCanvasElement;
  if (!canvas || !canvas.getContext) {
    return;
  }
  const queueDraw = () => requestAnimationFrame(() => draw(canvas, units));
  const resizeObserver = new ResizeObserver(queueDraw);
  resizeObserver.observe(canvas);
  const handleHover = ({
    clientX,
    clientY,
  }: { clientX: number; clientY: number }) => {
    const unit = findUnit(canvas, units, clientX, clientY);
    if (unit === hovered) {
      return;
    }
    if (unit?.filtered) {
      canvas.style.cursor = 'default';
      hovered = null;
    } else {
      if (clickable) {
        canvas.style.cursor = unit ? 'pointer' : 'default';
      }
      hovered = unit;
    }
    dirty = true;
    queueDraw();
  };
  const handleLeave = () => {
    if (!hovered) {
      return;
    }
    if (clickable) {
      canvas.style.cursor = 'default';
    }
    hovered = null;
    dirty = true;
    queueDraw();
  };

  const updateFilter = (filter: string) => {
    // Separate multiple different filter terms with spaces.
    const terms = filter.toLowerCase().split(/\s+/);
    for (const unit of units) {
      unit.filtered = !terms.every((term) =>
        checkFilterTermMatches(term, unit),
      );
    }
    unitsDirty = true;
    queueDraw();
  };
  const handleFilter = (evt: Event) => {
    if (
      evt.currentTarget === null ||
      !(evt.currentTarget instanceof HTMLInputElement)
    ) {
      return;
    }
    updateFilter(evt.currentTarget.value);
    const url = new URL(window.location.href);
    if (evt.currentTarget.value) {
      url.searchParams.set('filter', evt.currentTarget.value);
    } else {
      url.searchParams.delete('filter');
    }
    window.history.replaceState({}, '', url);
  };

  canvas.addEventListener('mousemove', (e) => {
    isTouch = false;
    handleHover(e);
  });
  canvas.addEventListener('mouseleave', handleLeave);
  canvas.addEventListener('touchmove', (e) => {
    isTouch = true;
    handleHover(e.touches[0]);
  });
  canvas.addEventListener('touchend', handleLeave);
  canvas.addEventListener('click', ({ clientX, clientY }) => {
    const unit = findUnit(canvas, units, clientX, clientY);
    if (!unit || !unit.name || unit.filtered || !clickable) {
      return;
    }
    const url = new URL(window.location.href);
    url.searchParams.set('unit', unit.name);
    url.searchParams.delete('filter');
    window.location.href = url.toString();
  });
  const filterInput = document.querySelector('input[name="filter"]');
  if (filterInput && filterInput instanceof HTMLInputElement) {
    updateFilter(filterInput.value); // Initialize on page load
    filterInput.addEventListener('input', handleFilter);
  }
  updatePixelRatio(queueDraw, false);
  draw(canvas, units);
};

let remove: (() => void) | null = null;

const updatePixelRatio = (redraw: () => void, now: boolean) => {
  if (remove != null) {
    remove();
  }
  const media = matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
  const cb = () => updatePixelRatio(redraw, true);
  media.addEventListener('change', cb);
  remove = () => {
    media.removeEventListener('change', cb);
  };
  if (now) {
    redraw();
  }
};

const SPECIAL_TERM_REGEXP = new RegExp(
  `^(>|<|>=|<=|=|==|!=)(\\d+(?:\\.\\d+)?)(%|${UNITS.join('|')})$`,
  'i',
);

const checkFilterTermMatches = (term: string, unit: Unit): boolean => {
  const match = term.match(SPECIAL_TERM_REGEXP);
  if (match) {
    // Filter based on match percent or size.
    const operator = match[1];
    const type = match[3];

    let lhs: number;
    let rhs: number;
    switch (type) {
      case '%':
        // Match percent
        lhs = unit.fuzzy_match_percent;
        rhs = Number.parseFloat(match[2]);
        break;
      default: {
        // Size unit, e.g. kB
        lhs = unit.total_code;
        rhs = Number.parseFloat(match[2]);
        let sizeUnitIndex = 0;
        while (sizeUnitIndex < UNITS.length - 1) {
          if (type.toLowerCase() === UNITS[sizeUnitIndex].toLowerCase()) {
            break;
          }
          rhs *= 1000.0;
          sizeUnitIndex += 1;
        }
        break;
      }
    }

    switch (operator) {
      case '>':
        return lhs > rhs;
      case '<':
        return lhs < rhs;
      case '>=':
        return lhs >= rhs;
      case '<=':
        return lhs <= rhs;
      case '=':
      case '==':
        return lhs === rhs;
      case '!=':
        return lhs !== rhs;
      default:
        return false;
    }
  }
  // Filter based on name.
  return unit.name.toLowerCase().includes(term);
};

window.drawTreemap = drawTreemap;

(function () {
  const url = new URL(window.location.href);
  const filterFromUrl = url.searchParams.get('filter');
  if (filterFromUrl) {
    const filterInput = document.querySelector('input[name="filter"]');
    if (filterInput && filterInput instanceof HTMLInputElement) {
      filterInput.value = filterFromUrl;
      filterInput.scrollIntoView();
    }
  }
})();
