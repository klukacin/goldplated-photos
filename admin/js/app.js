// Main App Controller
const app = {
  currentTab: 'albums',

  async init() {
    // Initialize utilities
    notifications.init();
    initModals();
    initPasswordToggles();

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

    // Clear cache
    document.getElementById('clear-cache-btn').addEventListener('click', async () => {
      const confirmed = await modal.confirm(
        'Clear Thumbnails',
        'This will delete all cached thumbnails. They will be regenerated on next access. Continue?'
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
