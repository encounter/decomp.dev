const platforms: Map<string, HTMLInputElement> = new Map();

function updateProjectVisibility() {
  const selectedPlatforms: string[] = [];
  let allSelected = true;
  for (const [platform, checkbox] of platforms.entries()) {
    if (checkbox.checked) {
      selectedPlatforms.push(platform);
    } else {
      allSelected = false;
    }
  }
  if (selectedPlatforms.length === 0) {
    allSelected = true;
    for (const cb of platforms.values()) {
      cb.checked = true;
    }
  }
  const url = new URL(window.location.href);
  if (allSelected) {
    if (url.searchParams.has('platform')) {
      url.searchParams.delete('platform');
      window.location.replace(url);
    }
  } else {
    url.searchParams.set('platform', selectedPlatforms.join(','));
    window.location.replace(url.toString().replace(/%2C/g, ','));
  }
}

document.querySelectorAll('.platform-item').forEach((item) => {
  const checkbox = item.querySelector(
    'input[type="checkbox"]',
  ) as HTMLInputElement | null;
  const button = item.querySelector('button') as HTMLButtonElement | null;
  if (!checkbox || !button) {
    return;
  }
  platforms.set(checkbox.value, checkbox);
  checkbox.addEventListener('click', (e) => e.stopPropagation());
  checkbox.addEventListener('change', () => updateProjectVisibility());
  button.addEventListener('click', () => {
    let allUnchecked = checkbox.checked;
    if (allUnchecked) {
      for (const cb of platforms.values()) {
        if (cb !== checkbox && cb.checked) {
          allUnchecked = false;
          break;
        }
      }
    }
    if (allUnchecked) {
      for (const cb of platforms.values()) {
        cb.checked = true;
      }
    } else {
      // Enable this checkbox and disable all others
      checkbox.checked = true;
      for (const cb of platforms.values()) {
        if (cb !== checkbox) {
          cb.checked = false;
        }
      }
    }
    updateProjectVisibility();
  });
});
