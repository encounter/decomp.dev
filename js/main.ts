const themeToggleButton = document.getElementById('theme-toggle');
if (themeToggleButton) {
    themeToggleButton.addEventListener('click', (e) => {
        const root = document.documentElement;
        const defaultDark = window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? true;
        const themeAttr = root.getAttribute('data-theme');
        const isDark = themeAttr === null ? defaultDark : themeAttr === 'dark';
        root.setAttribute('data-theme', isDark ? 'light' : 'dark');
        try {
            localStorage.setItem('theme', isDark ? 'light' : 'dark');
        } catch (e) {
            console.error('Failed to store theme', e);
        }
        e.preventDefault();
    });
}

let tooltipElem: HTMLElement | null = null;
const applyTooltip = (elem: HTMLElement) => {
    const text = elem.getAttribute('data-tooltip');
    if (!text) {
        return;
    }
    let tooltip = document.getElementById('tooltip');
    if (!tooltip) {
        tooltip = document.createElement('div');
        tooltip.id = 'tooltip';
        tooltip.addEventListener('transitionend', (e) => {
            if (e.propertyName === 'opacity' &&
                e.target instanceof HTMLElement &&
                e.target.style.opacity === '0') {
                e.target.remove();
            }
        });
        document.body.appendChild(tooltip);
    }
    tooltip.innerText = text;
    tooltip.style.opacity = '1';
    const rect = elem.getBoundingClientRect();
    let left = rect.left + window.scrollX + (rect.width - tooltip.offsetWidth) / 2;
    const docWidth = document.documentElement.clientWidth || document.body.clientWidth;
    if (left + tooltip.offsetWidth > docWidth) {
        left = docWidth - tooltip.offsetWidth;
    }
    if (left < 0) {
        left = 0;
    }
    let top = rect.top + window.scrollY - tooltip.offsetHeight;
    if (top < 0) {
        top = rect.bottom + window.scrollY;
    }
    tooltip.style.left = `${left}px`;
    tooltip.style.top = `${top}px`;
    const arrowLeft = rect.left + window.scrollX + rect.width / 2 - left;
    tooltip.style.setProperty('--arrow-left', `${arrowLeft}px`);
    tooltipElem = elem;
};
const removeTooltip = (elem: HTMLElement) => {
    if (elem !== tooltipElem) {
        return;
    }
    const tooltip = document.getElementById('tooltip');
    if (tooltip) {
        tooltip.style.opacity = '0';
    }
    tooltipElem = null;
};
document.addEventListener('mouseover', (e) => {
    if (e.target instanceof HTMLElement) {
        applyTooltip(e.target);
    }
}, { passive: true });
document.addEventListener('mouseout', (e) => {
    if (e.target instanceof HTMLElement) {
        removeTooltip(e.target);
    }
}, { passive: true });
document.addEventListener('touchstart', (e) => {
    if (e.target instanceof HTMLElement) {
        applyTooltip(e.target);
    }
}, { passive: true });
document.addEventListener('touchend', (e) => {
    if (e.target instanceof HTMLElement) {
        removeTooltip(e.target);
    }
}, { passive: true });
