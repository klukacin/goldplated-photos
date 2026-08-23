// Albums Module
const albums = {
  tree: [],
  selectedPath: null,
  bodyEditor: null,
  photosLoaded: false,
  videosLoaded: false,
  cacheLoaded: false,
  proofingLoaded: false,
  isNewAlbum: false,
  currentPhotoOrder: null,
  isDirty: false,

  async init() {
    await this.loadTree();
    this.initEventListeners();
    this.initEditorTabs();
    this.initBodyEditor();
    this.initVideoUpload();
    this.initDirtyTracking();
  },

  initDirtyTracking() {
    const form = document.getElementById('album-form');
    form.addEventListener('input', () => { this.isDirty = true; });
    form.addEventListener('change', () => { this.isDirty = true; });

    window.addEventListener('beforeunload', (e) => {
      if (this.isDirty) {
        e.preventDefault();
        e.returnValue = '';
      }
    });
  },

  markClean() {
    this.isDirty = false;
  },

  async confirmDiscardChanges() {
    if (!this.isDirty) return true;
    const confirmed = await modal.confirm(
      'Unsaved Changes',
      'You have unsaved changes in this album. Discard them?',
      'Discard'
    );
    if (confirmed) this.markClean();
    return confirmed;
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

        // Lazy load proofing submissions when Proofing tab is clicked
        if (tabName === 'proofing' && !this.proofingLoaded && this.selectedPath) {
          this.loadProofing(this.selectedPath);
          this.proofingLoaded = true;
        }
      });
    });

    // Proofing refresh button
    document.getElementById('refresh-proofing-btn').addEventListener('click', () => {
      if (this.selectedPath) this.loadProofing(this.selectedPath);
    });

    // Album cache buttons
    document.getElementById('refresh-album-cache-btn').addEventListener('click', () => {
      this.loadAlbumCacheStats();
    });

    document.getElementById('clear-album-cache-btn').addEventListener('click', async () => {
      if (!this.selectedPath) return;
      const confirmed = await modal.confirm(
        'Clear Album Thumbnails',
        'This will delete all cached thumbnails for this album. Continue?',
        'Clear'
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

  async loadProofing(albumPath) {
    const container = document.getElementById('proofing-submissions');
    container.innerHTML = '<p class="loading">Loading submissions...</p>';

    try {
      const submissions = await api.get(`/api/proofing/${albumPath}`);

      if (submissions.length === 0) {
        container.innerHTML = '<p class="empty-message">No client selections yet. Enable "Client Proofing" in Settings and share the album — selections will appear here.</p>';
        return;
      }

      container.innerHTML = '';
      submissions.forEach(sub => {
        const card = document.createElement('div');
        card.className = 'proofing-card';

        const header = document.createElement('div');
        header.className = 'proofing-card-header';

        const title = document.createElement('div');
        title.className = 'proofing-card-title';
        const when = new Date(sub.submittedAt).toLocaleString();
        title.textContent = `${sub.name || 'Anonymous'} — ${sub.selections.length} photo(s) — ${when}`;
        header.appendChild(title);

        const actions = document.createElement('div');
        actions.className = 'proofing-card-actions';

        const copyBtn = document.createElement('button');
        copyBtn.className = 'btn btn-sm';
        copyBtn.textContent = 'Copy list';
        copyBtn.addEventListener('click', async () => {
          const list = sub.selections.map(s => s.filename).join('\n');
          try {
            await navigator.clipboard.writeText(list);
            notifications.success('Filename list copied');
          } catch {
            notifications.error('Could not copy to clipboard');
          }
        });
        actions.appendChild(copyBtn);

        const csvBtn = document.createElement('button');
        csvBtn.className = 'btn btn-sm';
        csvBtn.textContent = 'CSV';
        csvBtn.addEventListener('click', () => {
          // Quoting is not enough on its own: a comment a client typed that
          // starts with = + - or @ is a formula, and Excel runs it when the
          // photographer opens the export. A leading apostrophe is the standard
          // way to say "this is text" and spreadsheets do not display it.
          const esc = (v) => {
            const s = String(v ?? '');
            const safe = /^[=+\-@\t\r]/.test(s) ? `'${s}` : s;
            return `"${safe.replace(/"/g, '""')}"`;
          };
          const rows = [
            ['filename', 'comment', 'client', 'submittedAt'].join(','),
            ...sub.selections.map(s => [esc(s.filename), esc(s.comment), esc(sub.name), esc(sub.submittedAt)].join(','))
          ];
          const blob = new Blob([rows.join('\n')], { type: 'text/csv' });
          const a = document.createElement('a');
          a.href = URL.createObjectURL(blob);
          a.download = `proofing-${albumPath.replace(/\//g, '_')}-${sub.id.replace(/\.json$/, '')}.csv`;
          a.click();
          URL.revokeObjectURL(a.href);
        });
        actions.appendChild(csvBtn);

        const deleteBtn = document.createElement('button');
        deleteBtn.className = 'btn btn-sm btn-danger';
        deleteBtn.textContent = 'Delete';
        deleteBtn.addEventListener('click', async () => {
          const confirmed = await modal.confirm(
            'Delete Submission',
            `Delete the selection from ${sub.name || 'Anonymous'} (${sub.selections.length} photos)?`,
            'Delete'
          );
          if (confirmed) {
            try {
              await api.delete(`/api/proofing/${encodeURIComponent(albumPath)}/file/${encodeURIComponent(sub.id)}`);
              notifications.success('Submission deleted');
              await this.loadProofing(albumPath);
              await this.loadTree();
            } catch (error) {
              notifications.error('Failed to delete submission: ' + error.message);
            }
          }
        });
        actions.appendChild(deleteBtn);

        header.appendChild(actions);
        card.appendChild(header);

        const grid = document.createElement('div');
        grid.className = 'proofing-thumbs';
        sub.selections.forEach(sel => {
          const cell = document.createElement('div');
          cell.className = 'proofing-thumb';

          const img = document.createElement('img');
          img.src = getPreviewThumbnailUrl(albumPath, sel.filename, 'small');
          img.alt = sel.filename;
          img.loading = 'lazy';
          // Only fall back to the original for formats a browser can paint —
          // a raw .heic here is a broken image on everything but Safari.
          img.onerror = () => {
            if (isBrowserDisplayableImage(sel.filename)) {
              img.src = getAlbumImageUrl(albumPath, sel.filename);
            } else {
              img.remove();
            }
          };
          cell.appendChild(img);

          const label = document.createElement('div');
          label.className = 'proofing-thumb-label';
          label.textContent = sel.filename;
          cell.appendChild(label);

          if (sel.comment) {
            const comment = document.createElement('div');
            comment.className = 'proofing-thumb-comment';
            comment.textContent = sel.comment;
            comment.title = sel.comment;
            cell.appendChild(comment);
          }

          grid.appendChild(cell);
        });
        card.appendChild(grid);

        container.appendChild(card);
      });
    } catch (error) {
      container.innerHTML = `<p class="error-message">Failed to load submissions: ${error.message}</p>`;
    }
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
          const confirmed = await modal.confirm('Delete Video', `Delete "${video.filename}"?`, 'Delete');
          if (confirmed) {
            try {
              await api.delete(`/api/videos/${encodeURIComponent(albumPath)}/file/${encodeURIComponent(video.filename)}`);
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

  initVideoUpload() {
    const input = document.getElementById('video-upload');
    const zone = document.getElementById('video-upload-zone');
    if (!input || !zone) return;

    input.addEventListener('change', async (e) => {
      if (e.target.files.length > 0) {
        await this.uploadVideos(e.target.files);
        e.target.value = '';
      }
    });

    zone.addEventListener('dragover', (e) => {
      e.preventDefault();
      zone.classList.add('drag-over');
    });

    zone.addEventListener('dragleave', () => {
      zone.classList.remove('drag-over');
    });

    zone.addEventListener('drop', async (e) => {
      e.preventDefault();
      zone.classList.remove('drag-over');
      const files = Array.from(e.dataTransfer.files).filter(f =>
        f.type.startsWith('video/') || /\.(mp4|webm|mov|avi|mkv|m4v)$/i.test(f.name)
      );
      if (files.length > 0) {
        await this.uploadVideos(files);
      }
    });
  },

  async uploadVideos(files) {
    if (!this.selectedPath) {
      notifications.error('Please select an album first');
      return;
    }

    const zone = document.getElementById('video-upload-zone');
    const originalText = zone.innerHTML;
    zone.innerHTML = `<p>Uploading ${files.length} video(s)... This can take a while.</p>`;

    try {
      const result = await api.upload(`/api/videos/${this.selectedPath}`, Array.from(files), 'videos');
      notifications.success(`Uploaded ${result.uploaded.length} video(s)`);
      if (result.renamed && result.renamed.length > 0) {
        notifications.warning(
          `${result.renamed.length} file(s) already existed and were saved under a new name (e.g. ${result.renamed[0].to})`
        );
      }
      await this.loadVideos(this.selectedPath);
      await this.loadTree();
    } catch (error) {
      notifications.error('Failed to upload videos: ' + error.message);
    } finally {
      zone.innerHTML = originalText;
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
      this.bodyEditor.on('change', (_cm, change) => {
        // setValue (programmatic populate) must not mark the form dirty
        if (change.origin !== 'setValue') this.isDirty = true;
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
          `Are you sure you want to delete "${this.selectedPath}"? This will also delete all photos in this album.`,
          'Delete'
        );
        if (confirmed) {
          await this.deleteAlbum(this.selectedPath);
        }
      }
    });

    // Rename / move album
    document.getElementById('rename-album-btn').addEventListener('click', () => {
      if (!this.selectedPath || this.isNewAlbum) return;
      document.getElementById('rename-album-path').value = this.selectedPath;
      modal.show('rename-album-modal');
    });

    document.getElementById('rename-album-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const newPath = document.getElementById('rename-album-path').value.trim();
      if (!newPath || newPath === this.selectedPath) {
        modal.hide('rename-album-modal');
        return;
      }
      try {
        const result = await api.post(`/api/album-rename/${this.selectedPath}`, { newPath });
        notifications.success(`Album moved to "${result.path}"`);
        modal.hide('rename-album-modal');
        this.markClean();
        await this.loadTree();
        await this.selectAlbum(result.path);
      } catch (error) {
        notifications.error('Failed to rename album: ' + error.message);
      }
    });

    // Generate share token (random secret for the shareable link)
    document.getElementById('generate-token-btn').addEventListener('click', async () => {
      const result = await api.get('/api/share-token');
      document.getElementById('album-share-token').value = result.shareToken;
      this.updateShareableLink();
      notifications.success('Share token generated. Save the album to activate the link.');
    });

    // Remove share token
    document.getElementById('remove-share-token-btn').addEventListener('click', () => {
      document.getElementById('album-share-token').value = '';
      this.updateShareableLink();
      notifications.warning('Share token removed. Save the album to deactivate the old link.');
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
    const shareToken = document.getElementById('album-share-token').value;
    if (path && shareToken) {
      // Prefer the configured production URL, fall back to the dev server
      const baseUrl = adminConfig.siteUrl || adminConfig.previewUrl;
      const shareLink = `${baseUrl}/photos/${path}?token=${shareToken}`;
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

  async reorderSibling(items, index, dir) {
    const target = index + dir;
    if (target < 0 || target >= items.length) return;
    const names = items.map(i => i.name);
    [names[index], names[target]] = [names[target], names[index]];
    const parent = items[index].path.split('/').slice(0, -1).join('/');
    try {
      const result = await api.post('/api/albums-reorder', { parent, order: names });
      if (result.skipped && result.skipped.length > 0) {
        notifications.warning(`Order not saved for folders without settings: ${result.skipped.join(', ')}`);
      }
      await this.loadTree();
    } catch (error) {
      notifications.error('Failed to reorder albums: ' + error.message);
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
      items.forEach((item, itemIndex) => {
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

        // Proofing submissions badge
        if (item.proofingCount > 0) {
          const proofingEl = document.createElement('span');
          proofingEl.className = 'tree-count tree-proofing-count';
          proofingEl.textContent = `♥${item.proofingCount}`;
          proofingEl.title = `${item.proofingCount} client selection${item.proofingCount > 1 ? 's' : ''}`;
          rowEl.appendChild(proofingEl);
        }

        // Reorder arrows (persist sibling order via `order` frontmatter)
        const reorderEl = document.createElement('span');
        reorderEl.className = 'tree-reorder';
        const upBtn = document.createElement('button');
        upBtn.textContent = '↑';
        upBtn.title = 'Move up';
        upBtn.disabled = itemIndex === 0;
        upBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          this.reorderSibling(items, itemIndex, -1);
        });
        const downBtn = document.createElement('button');
        downBtn.textContent = '↓';
        downBtn.title = 'Move down';
        downBtn.disabled = itemIndex === items.length - 1;
        downBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          this.reorderSibling(items, itemIndex, 1);
        });
        reorderEl.appendChild(upBtn);
        reorderEl.appendChild(downBtn);
        rowEl.appendChild(reorderEl);

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
    if (path !== this.selectedPath && !(await this.confirmDiscardChanges())) {
      return;
    }
    this.selectedPath = path;
    this.photosLoaded = false;
    this.videosLoaded = false;
    this.cacheLoaded = false;
    this.proofingLoaded = false;

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
      this.currentPhotoOrder = albumData.photoOrder || null;
      this.isNewAlbum = !!albumData.isNew;
      this.populateForm(albumData);
      this.markClean();

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

      // Update photo, video and proofing count badges (from tree data)
      const albumInTree = this.findAlbumInTree(path);
      const photoCount = albumInTree?.photoCount || 0;
      const videoCount = albumInTree?.videoCount || 0;
      document.getElementById('photo-count-badge').textContent = photoCount;
      document.getElementById('video-count-badge').textContent = videoCount;
      document.getElementById('proofing-count-badge').textContent = albumInTree?.proofingCount || 0;

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
    document.getElementById('album-share-token').value = data.shareToken || '';
    document.getElementById('album-sort').value = data.sort || 'date-desc';
    document.getElementById('album-style').value = data.style || 'grid';
    document.getElementById('album-tags').value = (data.tags || []).join(', ');
    document.getElementById('album-isCollection').checked = data.isCollection || false;
    document.getElementById('album-order').value = data.order || '';
    document.getElementById('album-hidden').checked = data.hidden || false;
    document.getElementById('album-allowDownload').checked = data.allowDownload || false;
    document.getElementById('album-proofing').checked = data.proofing || false;

    // Set body content
    if (this.bodyEditor) {
      this.bodyEditor.setValue(data.body || '');
      // Refresh CodeMirror to properly render after content change
      setTimeout(() => this.bodyEditor.refresh(), 10);
    } else {
      document.getElementById('album-body').value = data.body || '';
    }

    // Cover select: always reset and reflect the album's current thumbnail so
    // saving without opening the Photos tab never loses (or leaks) the cover.
    // The photos module later replaces this with the full file list.
    const thumbnailSelect = document.getElementById('album-thumbnail');
    thumbnailSelect.innerHTML = '<option value="">Auto (first photo)</option>';
    if (data.thumbnail) {
      const option = document.createElement('option');
      option.value = data.thumbnail;
      option.textContent = data.thumbnail;
      option.selected = true;
      thumbnailSelect.appendChild(option);
    }

    // Update shareable link
    this.updateShareableLink();
  },

  getFormData() {
    // null = clear the field on the server (merge removes null keys);
    // absent keys keep their existing value.
    const form = document.getElementById('album-form');
    const orderValue = form.querySelector('#album-order').value;
    const dateValue = form.querySelector('#album-date').value;
    const tagsValue = form.querySelector('#album-tags').value;

    return {
      title: form.querySelector('#album-title').value,
      description: form.querySelector('#album-description').value || null,
      password: form.querySelector('#album-password').value || null,
      token: form.querySelector('#album-token').value || undefined,
      shareToken: form.querySelector('#album-share-token').value || null,
      sort: form.querySelector('#album-sort').value,
      style: form.querySelector('#album-style').value,
      isCollection: form.querySelector('#album-isCollection').checked,
      hidden: form.querySelector('#album-hidden').checked,
      allowDownload: form.querySelector('#album-allowDownload').checked,
      proofing: form.querySelector('#album-proofing').checked,
      order: orderValue !== '' ? parseInt(orderValue) : null,
      date: dateValue ? parseDate(dateValue) : null,
      tags: tagsValue ? tagsValue.split(',').map(t => t.trim()).filter(t => t) : null,
      thumbnail: form.querySelector('#album-thumbnail').value || null,
      body: this.bodyEditor ? this.bodyEditor.getValue() : form.querySelector('#album-body').value
    };
  },

  async saveAlbum() {
    if (!this.selectedPath) return;

    try {
      const data = this.getFormData();

      if (this.isNewAlbum) {
        // Create new album (folder exists but no index.md)
        const result = await api.post('/api/albums', { path: this.selectedPath, ...data });
        notifications.success('Album created successfully');
        this.isNewAlbum = false;
        // Server may have sanitized the path (lowercase) — follow it
        if (result.path && result.path !== this.selectedPath) {
          await this.loadTree();
          await this.selectAlbum(result.path);
          return;
        }
      } else {
        await api.put(`/api/albums/${this.selectedPath}`, data);
        notifications.success('Album saved successfully');
      }

      this.markClean();

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
      // Use the server's (sanitized) path, not the raw input
      this.selectAlbum(result.path || path);
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
