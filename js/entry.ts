if (import.meta.env.DEV) {
  // For Rspack-loaded scripts, we need to set the nonce attribute on script tags
  const currentScript = document.currentScript;
  if (currentScript?.nonce) {
    __webpack_nonce__ = currentScript.nonce;
    // Additionally, for HMR support with Cross-Origin-Embedder-Policy (COEP),
    // we need to set the crossorigin attribute on script tags inserted by HMR
    const originalAppendChild = Element.prototype.appendChild;
    Element.prototype.appendChild = function <T extends Node>(node: T): T {
      if (node.nodeType === Node.ELEMENT_NODE && node.nodeName === 'SCRIPT') {
        const script = node as unknown as HTMLScriptElement;
        script.setAttribute('crossorigin', '');
      }
      return originalAppendChild.call(this, node) as T;
    };
  }
}

// Set the theme based on localStorage
let theme: string | null = null;
try {
  theme = localStorage.getItem('theme');
} catch (_) {}
if (theme) {
  document.documentElement.setAttribute('data-theme', theme);
}
