(function() {
    var menuBar = document.getElementById('mdbook-menu-bar') || document.getElementById('menu-bar');
    if (!menuBar) return;

    var rightButtons = menuBar.querySelector('.right-buttons');
    if (!rightButtons) return;

    if (rightButtons.querySelector('.torvyn-home-btn')) return;

    var homeLink = document.createElement('a');
    homeLink.href = '/torvyn/';
    homeLink.className = 'torvyn-home-btn';
    homeLink.setAttribute('aria-label', 'Back to Torvyn home');
    homeLink.innerHTML = '<svg fill="none" stroke="currentColor" stroke-width="2.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-4 0a1 1 0 01-1-1v-4a1 1 0 011-1h2a1 1 0 011 1v4a1 1 0 01-1 1h-2z"/></svg>Home';

    rightButtons.insertBefore(homeLink, rightButtons.firstChild);
})();
