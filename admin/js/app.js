// Main App Controller
const app = {
  currentTab: 'albums',

  async init() {
    // Initialize utilities
    notifications.init();
    initModals();
    initPasswordToggles();
    await loadAdminConfig();

    // Initialize tabs
    this.initTabs();

    // Initialize modules
    await albums.init();
    photos.init();
    await home.init();
    this.initAssets();
    this.initTools();
  },

  initTabs() {
    const tabs = document.querySelectorAll('.tab');
    const contents = document.querySelectorAll('.tab-content');

    tabs.forEach(tab => {
      tab.addEventListener('click', () => {
        const tabName = tab.dataset.tab;

        // Update tab buttons
        tabs.forEach(t => t.classList.remove('active'));
        tab.classList.add('active');

        // Update content
        contents.forEach(c => c.classList.remove('active'));
        document.getElementById(`${tabName}-tab`).classList.add('active');

        // Load content if needed
        if (tabName === 'home' && this.currentTab !== 'home') {
          home.load();
        } else if (tabName === 'tools' && this.currentTab !== 'tools') {
          this.loadCacheStats();
        }

        this.currentTab = tabName;
      });
    });
  },

  initAssets() {
    // All asset handling now in home.js
  },

  async loadAssets() {
    // All asset handling now in home.js
  },

  initTools() {
    // Refresh stats button
    document.getElementById('refresh-stats-btn').addEventListener('click', () => {
      this.loadCacheStats();
    });

    this.initScriptRunner();

    // Clear cache
    document.getElementById('clear-cache-btn').addEventListener('click', async () => {
      const confirmed = await modal.confirm(
        'Clear Thumbnails',
        'This will delete all cached thumbnails. They will be regenerated on next access. Continue?',
        'Clear'
      );
      if (confirmed) {
        const btn = document.getElementById('clear-cache-btn');
        btn.classList.add('loading');
        try {
          await api.delete('/api/cache/thumbnails');
          notifications.success('Thumbnail cache cleared');
          await this.loadCacheStats();
        } catch (error) {
          notifications.error('Failed to clear cache: ' + error.message);
        } finally {
          btn.classList.remove('loading');
        }
      }
    });
  },

  async initScriptRunner() {
    const buttonsEl = document.getElementById('runner-buttons');
    const outputEl = document.getElementById('runner-output');
    const statusEl = document.getElementById('runner-status');
    if (!buttonsEl) return;

    let scripts = [];
    try {
      const data = await api.get('/api/tools/scripts');
      scripts = data.scripts;
    } catch {
      buttonsEl.innerHTML = '<p style="color: var(--text-secondary);">Script runner unavailable.</p>';
      return;
    }

    const setRunning = (running, label = '') => {
      buttonsEl.querySelectorAll('button').forEach(b => { b.disabled = running; });
      statusEl.textContent = running ? `Running: ${label}…` : '';
    };

    scripts.forEach(script => {
      const btn = document.createElement('button');
      btn.className = 'btn' + (script.confirm ? ' btn-warning' : '');
      btn.textContent = script.label;
      btn.addEventListener('click', async () => {
        if (script.confirm) {
          const confirmed = await modal.confirm(
            script.label,
            `Run "${script.label}" now? Watch the output below.`,
            'Run'
          );
          if (!confirmed) return;
        }

        outputEl.textContent = '';
        outputEl.classList.remove('hidden');
        setRunning(true, script.label);

        const source = new EventSource(`/api/tools/run/${script.id}`);
        const append = (text) => {
          outputEl.textContent += text;
          outputEl.scrollTop = outputEl.scrollHeight;
        };
        source.addEventListener('output', (e) => append(JSON.parse(e.data)));
        source.addEventListener('done', (e) => {
          const { code } = JSON.parse(e.data);
          append(`\n— finished with exit code ${code} —\n`);
          source.close();
          setRunning(false);
          if (code === 0) {
            notifications.success(`${script.label} finished`);
          } else {
            notifications.error(`${script.label} failed (exit code ${code})`);
          }
        });
        source.onerror = () => {
          // Connection failed (409 lock or server error) — EventSource can't
          // expose the status, so report generically
          if (source.readyState === EventSource.CLOSED) return;
          source.close();
          setRunning(false);
          append('\n— connection lost (is another script already running?) —\n');
          notifications.error(`${script.label}: connection lost`);
        };
      });
      buttonsEl.appendChild(btn);
    });
  },

  async loadCacheStats() {
    try {
      const stats = await api.get('/api/cache/stats');
      document.getElementById('stat-small').textContent = stats.small.toLocaleString();
      document.getElementById('stat-medium').textContent = stats.medium.toLocaleString();
      document.getElementById('stat-large').textContent = stats.large.toLocaleString();
      document.getElementById('stat-total').textContent = stats.total.toLocaleString();
    } catch (error) {
      console.error('Failed to load cache stats:', error);
    }
  }
};

// Initialize app when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
  app.init();
});
