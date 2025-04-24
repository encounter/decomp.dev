document.querySelectorAll('form[data-loading]').forEach((form) => {
    const loadingText = form.getAttribute('data-loading');
    const submitButton = form.querySelector('[type="submit"]');
    if (!submitButton || !(submitButton instanceof HTMLButtonElement)) {
        return;
    }
    form.addEventListener('submit', () => {
        submitButton.disabled = true;
        submitButton.setAttribute('aria-busy', 'true');
        submitButton.innerText = loadingText;
    });
});
