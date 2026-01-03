// Albums Module
const albums = {
  tree: [],
  selectedPath: null,
  bodyEditor: null,
  photosLoaded: false,
  videosLoaded: false,
  cacheLoaded: false,

  async init() {
    await this.loadTree();
    this.initEventListeners();
    this.initEditorTabs();
    this.initBodyEditor();
  },

  initEditorTabs() {
    const tabs = document.querySelectorAll('.editor-tab');
    const contents = document.querySelectorAll('.editor-tab-content');

    tabs.forEach(tab => {
      tab.addEventListener('click', () => {
        const tabName = tab.dataset.editorTab;

        // Update tabs
        tabs.forEach(t => t.classList.remove('active'));
        tab.classList.add('active');

        // Update content
        contents.forEach(c => c.classList.remove('active'));
        document.getElementById(`${tabName}-panel`).classList.add('active');

        // Lazy load photos when Photos tab is clicked
        if (tabName === 'photos' && !this.photosLoaded && this.selectedPath) {
          photos.loadPhotos(this.selectedPath, this.currentThumbnail);
          this.photosLoaded = true;
        }

        // Lazy load videos when Videos tab is clicked
        if (tabName === 'videos' && !this.videosLoaded && this.selectedPath) {
          this.loadVideos(this.selectedPath);
          this.videosLoaded = true;
        }

        // Lazy load cache stats when Cache tab is clicked
        if (tabName === 'cache' && !this.cacheLoaded && this.selectedPath) {
          this.loadAlbumCacheStats();
          this.cacheLoaded = true;
        }
      });
    });

    // Album cache buttons
    document.getElementById('refresh-album-cache-btn').addEventListener('click', () => {
      this.loadAlbumCacheStats();
    });

    document.getElementById('clear-album-cache-btn').addEventListener('click', async () => {
      if (!this.selectedPath) return;
      const confirmed = await modal.confirm(
        'Clear Album Thumbnails',
        'This will delete all cached thumbnails for this album. Continue?'
      );
      if (confirmed) {
        const btn = document.getElementById('clear-album-cache-btn');
        btn.classList.add('loading');
        try {
          const result = await api.delete(`/api/cache/album/${this.selectedPath}`);
          notifications.success(`Cleared ${result.deleted} thumbnails`);
          this.loadAlbumCacheStats();
        } catch (error) {
          notifications.error('Failed to clear cache: ' + error.message);
        } finally {
          btn.classList.remove('loading');
        }
      }
    });
  },

  async loadAlbumCacheStats() {
    if (!this.selectedPath) return;
    try {
      const stats = await api.get(`/api/cache/album/${this.selectedPath}`);
      document.getElementById('album-stat-small').textContent = stats.small.toLocaleString();
      document.getElementById('album-stat-medium').textContent = stats.medium.toLocaleString();
      document.getElementById('album-stat-large').textContent = stats.large.toLocaleString();
      document.getElementById('album-stat-total').textContent = stats.total.toLocaleString();
    } catch (error) {
      console.error('Failed to load album cache stats:', error);
    }
  },

  async loadVideos(albumPath) {
    const grid = document.getElementById('video-grid');
    if (!grid) return;

    grid.innerHTML = '<p class="loading">Loading videos...</p>';

    try {
      const videos = await api.get(`/api/videos/${albumPath}`);

      if (videos.length === 0) {
        grid.innerHTML = '<p class="empty-message">No videos in this album</p>';
        return;
      }

      grid.innerHTML = '';

      videos.forEach(video => {
        const item = document.createElement('div');
        item.className = 'video-item';
        item.dataset.filename = video.filename;

        // Video with actual player
        const videoUrl = `/albums/${albumPath}/${video.filename}`;
        item.innerHTML = `
          <div class="video-preview" data-video-url="${videoUrl}">
            <video class="video-player-preview" preload="metadata" muted>
              <source src="${videoUrl}" type="video/${video.filename.split('.').pop().toLowerCase() === 'mov' ? 'quicktime' : 'mp4'}">
            </video>
            <div class="video-play-overlay">
              <svg width="48" height="48" viewBox="0 0 24 24" fill="currentColor">
                <path d="M8 5v14l11-7z"/>
              </svg>
            </div>
            <div class="video-filename">${video.filename}</div>
          </div>
          <div class="video-info">
            <span class="video-size">${this.formatFileSize(video.size)}</span>
          </div>
          <div class="video-actions">
            <button class="btn btn-sm btn-danger delete-video" title="Delete video">Delete</button>
          </div>
        `;

        // Click to play/pause the video
        const preview = item.querySelector('.video-preview');
        const videoEl = item.querySelector('.video-player-preview');
        const overlay = item.querySelector('.video-play-overlay');

        preview.addEventListener('click', () => {
          if (videoEl.paused) {
            // Pause all other videos first
            document.querySelectorAll('.video-player-preview').forEach(v => {
              if (v !== videoEl) {
                v.pause();
                v.closest('.video-preview').querySelector('.video-play-overlay').style.display = '';
              }
            });

            videoEl.muted = false;
            videoEl.controls = true;
            videoEl.play();
            overlay.style.display = 'none';
            item.classList.add('playing');
          } else {
            videoEl.pause();
            overlay.style.display = '';
            item.classList.remove('playing');
          }
        });

        // Show overlay when video ends
        videoEl.addEventListener('ended', () => {
          overlay.style.display = '';
          item.classList.remove('playing');
        });

        // Handle video error (unsupported codec)
        const sourceEl = videoEl.querySelector('source');
        let hasError = false;
        let errorTimeout = null;

        const showError = () => {
          if (hasError) return;
          hasError = true;
          if (errorTimeout) clearTimeout(errorTimeout);
          preview.innerHTML = `
            <div class="video-error">
              <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <circle cx="12" cy="12" r="10"></circle>
                <line x1="12" y1="8" x2="12" y2="12"></line>
                <line x1="12" y1="16" x2="12.01" y2="16"></line>
              </svg>
              <p>Format not supported</p>
              <a href="${videoUrl}" download="${video.filename}" class="btn btn-sm">Download</a>
            </div>
            <div class="video-filename">${video.filename}</div>
          `;
        };

        // Listen on BOTH video AND source elements
        videoEl.addEventListener('error', showError, { once: true });
        if (sourceEl) sourceEl.addEventListener('error', showError, { once: true });

        // Timeout fallback
        errorTimeout = setTimeout(() => {
          if (videoEl.readyState === 0 && !hasError) {
            showError();
          }
        }, 3000);

        // Cancel timeout if video loads
        videoEl.addEventListener('loadedmetadata', () => {
          if (errorTimeout) clearTimeout(errorTimeout);
        }, { once: true });

        // Delete button handler
        item.querySelector('.delete-video').addEventListener('click', async (e) => {
          e.stopPropagation(); // Don't trigger video play
          const confirmed = await modal.confirm('Delete Video', `Delete "${video.filename}"?`);
          if (confirmed) {
            try {
              await api.delete(`/api/photos/${albumPath}/${video.filename}`);
              item.remove();
              notifications.success('Video deleted');
              // Reload tree to update counts
              await this.loadTree();
            } catch (error) {
              notifications.error('Failed to delete video: ' + error.message);
            }
          }
        });

        grid.appendChild(item);
      });
    } catch (error) {
      grid.innerHTML = `<p class="error-message">Failed to load videos: ${error.message}</p>`;
    }
  },

  formatFileSize(bytes) {
    if (!bytes) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB'];
    let i = 0;
    while (bytes >= 1024 && i < units.length - 1) {
      bytes /= 1024;
      i++;
    }
    return `${bytes.toFixed(1)} ${units[i]}`;
  },

  initBodyEditor() {
    const textarea = document.getElementById('album-body');
    if (textarea && typeof CodeMirror !== 'undefined') {
      this.bodyEditor = CodeMirror.fromTextArea(textarea, {
        mode: 'markdown',
        theme: 'monokai',
        lineWrapping: true,
        lineNumbers: true
      });
      // Refresh after initialization to ensure proper rendering
      setTimeout(() => this.bodyEditor.refresh(), 100);
    }
  },

  initEventListeners() {
    // New album button
    document.getElementById('new-album-btn').addEventListener('click', () => {
      modal.show('new-album-modal');
    });

    // New album form
    document.getElementById('new-album-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      await this.createAlbum(e.target);
    });

    // Save album
    document.getElementById('save-album-btn').addEventListener('click', () => {
      this.saveAlbum();
    });

    // Delete album
    document.getElementById('delete-album-btn').addEventListener('click', async () => {
      if (this.selectedPath) {
        const confirmed = await modal.confirm(
          'Delete Album',
          `Are you sure you want to delete "${this.selectedPath}"? This will also delete all photos in this album.`
        );
        if (confirmed) {
          await this.deleteAlbum(this.selectedPath);
        }
      }
    });

    // Generate token
    document.getElementById('generate-token-btn').addEventListener('click', async () => {
      const token = await generateToken();
      document.getElementById('album-token').value = token;
      this.updateShareableLink();
    });

    // Copy shareable link
    document.getElementById('copy-link-btn').addEventListener('click', async () => {
      const link = document.getElementById('album-share-link').value;
      if (link) {
        try {
          await navigator.clipboard.writeText(link);
          notifications.success('Link copied to clipboard!');
        } catch (err) {
          // Fallback for older browsers
          const input = document.getElementById('album-share-link');
          input.select();
          document.execCommand('copy');
          notifications.success('Link copied to clipboard!');
        }
      }
    });
  },

  updateShareableLink() {
    const path = document.getElementById('album-path').value;
    const token = document.getElementById('album-token').value;
    if (path && token) {
      const baseUrl = window.location.origin.replace(':4444', ':4321'); // Use dev server port
      const shareLink = `${baseUrl}/photos/${path}?token=${token}`;
      document.getElementById('album-share-link').value = shareLink;
    } else {
      document.getElementById('album-share-link').value = '';
    }
  },

  async loadTree() {
    try {
      this.tree = await api.get('/api/albums');
      this.renderTree();
    } catch (error) {
      notifications.error('Failed to load albums: ' + error.message);
    }
  },

  renderTree() {
    const container = document.getElementById('album-tree');
    container.innerHTML = '';

    if (this.tree.length === 0) {
      container.innerHTML = '<p style="padding: 1rem; color: var(--text-secondary);">No albums yet. Create one!</p>';
      return;
    }

    const renderItems = (items, parent) => {
      items.forEach(item => {
        const itemEl = document.createElement('div');
        itemEl.className = 'tree-item';

        const rowEl = document.createElement('div');
        rowEl.className = 'tree-row';
        rowEl.dataset.path = item.path;  // Store path for selection lookup
        if (item.path === this.selectedPath) {
          rowEl.classList.add('selected');
        }

        // Toggle arrow - start collapsed
        const toggleEl = document.createElement('span');
        toggleEl.className = 'tree-toggle';
        if (item.children && item.children.length > 0) {
          toggleEl.textContent = '►';  // Start collapsed
          toggleEl.addEventListener('click', (e) => {
            e.stopPropagation();
            const childrenEl = itemEl.querySelector('.tree-children');
            if (childrenEl) {
              childrenEl.classList.toggle('collapsed');
              toggleEl.textContent = childrenEl.classList.contains('collapsed') ? '►' : '▼';
            }
          });
        }
        rowEl.appendChild(toggleEl);

        // Icon
        const iconEl = document.createElement('span');
        iconEl.className = 'tree-icon';
        if (item.meta?.password) {
          iconEl.textContent = '🔒';
        } else if (item.isCollection) {
          iconEl.textContent = '📁';
        } else {
          iconEl.textContent = '🖼️';
        }
        rowEl.appendChild(iconEl);

        // Label
        const labelEl = document.createElement('span');
        labelEl.className = 'tree-label';
        labelEl.textContent = item.meta?.title || item.name;
        rowEl.appendChild(labelEl);

        // Photo count
        if (item.photoCount > 0) {
          const countEl = document.createElement('span');
          countEl.className = 'tree-count';
          countEl.textContent = item.photoCount;
          rowEl.appendChild(countEl);
        }

        // Video count (in red)
        if (item.videoCount > 0) {
          const videoCountEl = document.createElement('span');
          videoCountEl.className = 'tree-count tree-video-count';
          videoCountEl.textContent = item.videoCount;
          videoCountEl.title = `${item.videoCount} video${item.videoCount > 1 ? 's' : ''}`;
          rowEl.appendChild(videoCountEl);
        }

        // Click to select
        rowEl.addEventListener('click', () => {
          this.selectAlbum(item.path);
        });

        itemEl.appendChild(rowEl);

        // Children - start collapsed
        if (item.children && item.children.length > 0) {
          const childrenEl = document.createElement('div');
          childrenEl.className = 'tree-children collapsed';
          renderItems(item.children, childrenEl);
          itemEl.appendChild(childrenEl);
        }

        parent.appendChild(itemEl);
      });
    };

    renderItems(this.tree, container);
  },

  async selectAlbum(path) {
    this.selectedPath = path;
    this.photosLoaded = false;
    this.videosLoaded = false;
    this.cacheLoaded = false;

    // Update selection without re-rendering (preserves expand/collapse state)
    document.querySelectorAll('#album-tree .tree-row').forEach(row => {
      if (row.dataset.path === path) {
        row.classList.add('selected');
      } else {
        row.classList.remove('selected');
      }
    });

    // Reset to Settings tab
    document.querySelectorAll('.editor-tab').forEach(t => t.classList.remove('active'));
    document.querySelectorAll('.editor-tab-content').forEach(c => c.classList.remove('active'));
    document.querySelector('[data-editor-tab="settings"]').classList.add('active');
    document.getElementById('settings-panel').classList.add('active');

    // Clear photo and video grids
    document.getElementById('photo-grid').innerHTML = '';
    document.getElementById('video-grid').innerHTML = '';

    try {
      const albumData = await api.get(`/api/albums/${path}`);
      this.currentThumbnail = albumData.thumbnail;
      this.populateForm(albumData);

      document.getElementById('album-editor').classList.remove('hidden');
      document.getElementById('album-placeholder').classList.add('hidden');

      // Refresh CodeMirror after editor becomes visible
      if (this.bodyEditor) {
        setTimeout(() => this.bodyEditor.refresh(), 50);
      }

      // Show different title for new vs existing albums
      const titleEl = document.getElementById('album-editor-title');
      const saveBtn = document.getElementById('save-album-btn');
      if (albumData.isNew) {
        titleEl.textContent = `Configure: ${albumData.title || path}`;
        saveBtn.textContent = 'Create Album';
      } else {
        titleEl.textContent = albumData.title || path;
        saveBtn.textContent = 'Save';
      }

      // Update photo and video count badges (from tree data)
      const albumInTree = this.findAlbumInTree(path);
      const photoCount = albumInTree?.photoCount || 0;
      const videoCount = albumInTree?.videoCount || 0;
      document.getElementById('photo-count-badge').textContent = photoCount;
      document.getElementById('video-count-badge').textContent = videoCount;

    } catch (error) {
      notifications.error('Failed to load album: ' + error.message);
    }
  },

  findAlbumInTree(path, tree = this.tree) {
    for (const item of tree) {
      if (item.path === path) return item;
      if (item.children && item.children.length > 0) {
        const found = this.findAlbumInTree(path, item.children);
        if (found) return found;
      }
    }
    return null;
  },

  populateForm(data) {
    document.getElementById('album-path').value = data.path || '';
    document.getElementById('album-title').value = data.title || '';
    document.getElementById('album-description').value = data.description || '';
    document.getElementById('album-date').value = formatDate(data.date);
    document.getElementById('album-password').value = data.password || '';
    document.getElementById('album-token').value = data.token || '';
    document.getElementById('album-sort').value = data.sort || 'date-desc';
    document.getElementById('album-style').value = data.style || 'grid';
    document.getElementById('album-tags').value = (data.tags || []).join(', ');
    document.getElementById('album-isCollection').checked = data.isCollection || false;
    document.getElementById('album-allowAnonymous').checked = data.allowAnonymous !== false;
    document.getElementById('album-order').value = data.order || '';
    document.getElementById('album-hidden').checked = data.hidden || false;
    document.getElementById('album-allowDownload').checked = data.allowDownload || false;

    // Set body content
    if (this.bodyEditor) {
      this.bodyEditor.setValue(data.body || '');
      // Refresh CodeMirror to properly render after content change
      setTimeout(() => this.bodyEditor.refresh(), 10);
    } else {
      document.getElementById('album-body').value = data.body || '';
    }

    // Thumbnail will be populated by photos module

    // Update shareable link
    this.updateShareableLink();
  },

  getFormData() {
    const form = document.getElementById('album-form');
    const data = {
      title: form.querySelector('#album-title').value,
      description: form.querySelector('#album-description').value || undefined,
      password: form.querySelector('#album-password').value || undefined,
      token: form.querySelector('#album-token').value,
      sort: form.querySelector('#album-sort').value,
      style: form.querySelector('#album-style').value,
      isCollection: form.querySelector('#album-isCollection').checked,
      allowAnonymous: form.querySelector('#album-allowAnonymous').checked,
      hidden: form.querySelector('#album-hidden').checked,
      allowDownload: form.querySelector('#album-allowDownload').checked,
      body: this.bodyEditor ? this.bodyEditor.getValue() : form.querySelector('#album-body').value
    };

    // Order (numeric, optional)
    const orderValue = form.querySelector('#album-order').value;
    if (orderValue !== '') {
      data.order = parseInt(orderValue);
    }

    // Date
    const dateValue = form.querySelector('#album-date').value;
    if (dateValue) {
      data.date = parseDate(dateValue);
    }

    // Tags
    const tagsValue = form.querySelector('#album-tags').value;
    if (tagsValue) {
      data.tags = tagsValue.split(',').map(t => t.trim()).filter(t => t);
    }

    // Thumbnail
    const thumbnailValue = form.querySelector('#album-thumbnail').value;
    if (thumbnailValue) {
      data.thumbnail = thumbnailValue;
    }

    return data;
  },

  async saveAlbum() {
    if (!this.selectedPath) return;

    try {
      const data = this.getFormData();
      const isNew = document.getElementById('save-album-btn').textContent === 'Create Album';

      if (isNew) {
        // Create new album (folder exists but no index.md)
        await api.post('/api/albums', { path: this.selectedPath, ...data });
        notifications.success('Album created successfully');
      } else {
        await api.put(`/api/albums/${this.selectedPath}`, data);
        notifications.success('Album saved successfully');
      }

      // Reset button text
      document.getElementById('save-album-btn').textContent = 'Save';

      await this.loadTree();
    } catch (error) {
      notifications.error('Failed to save album: ' + error.message);
    }
  },

  async createAlbum(form) {
    const path = form.querySelector('#new-album-path').value.trim();
    const title = form.querySelector('#new-album-title').value.trim();
    const isCollection = form.querySelector('#new-album-isCollection').checked;

    if (!path) {
      notifications.error('Path is required');
      return;
    }

    try {
      const result = await api.post('/api/albums', {
        path,
        title: title || undefined,
        isCollection
      });

      notifications.success('Album created successfully');
      modal.hide('new-album-modal');
      form.reset();

      await this.loadTree();
      this.selectAlbum(path);
    } catch (error) {
      notifications.error('Failed to create album: ' + error.message);
    }
  },

  async deleteAlbum(path) {
    try {
      await api.delete(`/api/albums/${path}`);
      notifications.success('Album deleted');

      this.selectedPath = null;
      document.getElementById('album-editor').classList.add('hidden');
      document.getElementById('album-placeholder').classList.remove('hidden');

      await this.loadTree();
    } catch (error) {
      notifications.error('Failed to delete album: ' + error.message);
    }
  }
};
