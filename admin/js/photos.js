// Photos Module — grid, upload (with progress), selection/bulk ops,
// drag & drop reordering and click-to-preview.
const photos = {
  currentAlbumPath: null,
  photoList: [],
  currentThumbnail: null,
  selectionMode: false,
  selected: new Set(),
  dragIndex: null,

  init() {
    this.initEventListeners();
    this.initDragDrop();
    this.initSelectionToolbar();
    this.initPreviewModal();
    this.initMoveModal();
  },

  initEventListeners() {
    // Photo upload
    document.getElementById('photo-upload').addEventListener('change', async (e) => {
      if (e.target.files.length > 0) {
        await this.uploadPhotos(e.target.files);
        e.target.value = ''; // Reset input
      }
    });
  },

  initDragDrop() {
    const uploadZone = document.getElementById('upload-zone');

    uploadZone.addEventListener('dragover', (e) => {
      e.preventDefault();
      uploadZone.classList.add('drag-over');
    });

    uploadZone.addEventListener('dragleave', () => {
      uploadZone.classList.remove('drag-over');
    });

    uploadZone.addEventListener('drop', async (e) => {
      e.preventDefault();
      uploadZone.classList.remove('drag-over');

      const files = Array.from(e.dataTransfer.files).filter(f =>
        f.type.startsWith('image/')
      );

      if (files.length > 0) {
        await this.uploadPhotos(files);
      }
    });
  },

  initSelectionToolbar() {
    document.getElementById('select-photos-btn').addEventListener('click', () => {
      this.setSelectionMode(!this.selectionMode);
    });
    document.getElementById('select-all-btn').addEventListener('click', () => {
      if (this.selected.size === this.photoList.length) {
        this.selected.clear();
      } else {
        this.photoList.forEach(p => this.selected.add(p.filename));
      }
      this.renderPhotos();
    });
    document.getElementById('bulk-delete-btn').addEventListener('click', () => this.bulkDelete());
    document.getElementById('bulk-move-btn').addEventListener('click', () => {
      if (this.selected.size > 0) this.openMoveModal();
    });
    document.getElementById('bulk-cover-btn').addEventListener('click', () => {
      if (this.selected.size === 1) {
        this.setAsCover([...this.selected][0]);
        this.setSelectionMode(false);
      }
    });
  },

  setSelectionMode(on) {
    this.selectionMode = on;
    this.selected.clear();
    document.getElementById('bulk-toolbar').classList.toggle('hidden', !on);
    document.getElementById('select-photos-btn').textContent = on ? 'Done' : 'Select';
    this.renderPhotos();
  },

  updateBulkToolbar() {
    const n = this.selected.size;
    document.getElementById('bulk-count').textContent = `${n} selected`;
    document.getElementById('bulk-delete-btn').disabled = n === 0;
    document.getElementById('bulk-move-btn').disabled = n === 0;
    document.getElementById('bulk-cover-btn').disabled = n !== 1;
  },

  async loadPhotos(albumPath, currentThumbnail = null) {
    this.currentAlbumPath = albumPath;
    this.currentThumbnail = currentThumbnail;
    this.setSelectionMode(false);

    try {
      this.photoList = await api.get(`/api/photos/${albumPath}`);
      this.applySavedOrder();
      this.renderPhotos();
      this.updateThumbnailSelect();
      // Update photo count badge
      document.getElementById('photo-count-badge').textContent = this.photoList.length;
    } catch (error) {
      notifications.error('Failed to load photos: ' + error.message);
    }
  },

  // Order the list by the album's saved photoOrder (unlisted files last, by name)
  applySavedOrder() {
    const order = albums.currentPhotoOrder;
    if (!Array.isArray(order) || order.length === 0) return;
    const index = new Map(order.map((f, i) => [f, i]));
    this.photoList.sort((a, b) => {
      const ia = index.has(a.filename) ? index.get(a.filename) : Infinity;
      const ib = index.has(b.filename) ? index.get(b.filename) : Infinity;
      if (ia !== ib) return ia - ib;
      return a.filename.localeCompare(b.filename);
    });
  },

  renderPhotos() {
    const grid = document.getElementById('photo-grid');
    grid.innerHTML = '';

    if (this.photoList.length === 0) {
      grid.innerHTML = '<p style="color: var(--text-secondary); grid-column: 1/-1;">No photos in this album yet.</p>';
      this.updateBulkToolbar();
      return;
    }

    this.photoList.forEach((photo, index) => {
      const item = document.createElement('div');
      item.className = 'photo-item';
      item.dataset.index = index;
      if (photo.filename === this.currentThumbnail) {
        item.classList.add('is-cover');
      }
      if (this.selected.has(photo.filename)) {
        item.classList.add('selected');
      }

      const img = document.createElement('img');
      // Use small thumbnail for faster loading in admin
      img.src = `${adminConfig.previewUrl}/api/thumbnail?path=${encodeURIComponent(this.currentAlbumPath + '/' + photo.filename)}&size=small`;
      img.alt = photo.filename;
      img.loading = 'lazy';
      img.draggable = false;
      // Fallback to original if thumbnail fails
      img.onerror = () => {
        img.src = getAlbumImageUrl(this.currentAlbumPath, photo.filename);
      };
      item.appendChild(img);

      if (this.selectionMode) {
        const check = document.createElement('span');
        check.className = 'photo-select-check';
        check.textContent = this.selected.has(photo.filename) ? '✓' : '';
        item.appendChild(check);

        item.addEventListener('click', () => {
          if (this.selected.has(photo.filename)) {
            this.selected.delete(photo.filename);
          } else {
            this.selected.add(photo.filename);
          }
          this.renderPhotos();
        });
      } else {
        // Click opens the preview
        item.addEventListener('click', () => this.openPreview(photo));

        const overlay = document.createElement('div');
        overlay.className = 'photo-overlay';

        const setCoverBtn = document.createElement('button');
        setCoverBtn.textContent = 'Cover';
        setCoverBtn.title = 'Set as cover photo';
        setCoverBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          this.setAsCover(photo.filename);
        });
        overlay.appendChild(setCoverBtn);

        const deleteBtn = document.createElement('button');
        deleteBtn.textContent = 'Delete';
        deleteBtn.style.background = 'var(--danger)';
        deleteBtn.addEventListener('click', async (e) => {
          e.stopPropagation();
          const confirmed = await modal.confirm(
            'Delete Photo',
            `Are you sure you want to delete "${photo.filename}"?`,
            'Delete'
          );
          if (confirmed) {
            await this.deletePhoto(photo.filename);
          }
        });
        overlay.appendChild(deleteBtn);

        item.appendChild(overlay);

        // Drag & drop reordering (disabled in selection mode)
        item.draggable = true;
        item.addEventListener('dragstart', (e) => {
          this.dragIndex = index;
          item.classList.add('dragging');
          e.dataTransfer.effectAllowed = 'move';
        });
        item.addEventListener('dragend', () => {
          item.classList.remove('dragging');
          grid.querySelectorAll('.drag-over-item').forEach(el => el.classList.remove('drag-over-item'));
        });
        item.addEventListener('dragover', (e) => {
          e.preventDefault();
          e.dataTransfer.dropEffect = 'move';
          item.classList.add('drag-over-item');
        });
        item.addEventListener('dragleave', () => {
          item.classList.remove('drag-over-item');
        });
        item.addEventListener('drop', async (e) => {
          e.preventDefault();
          item.classList.remove('drag-over-item');
          if (this.dragIndex === null || this.dragIndex === index) return;
          const [moved] = this.photoList.splice(this.dragIndex, 1);
          this.photoList.splice(index, 0, moved);
          this.dragIndex = null;
          this.renderPhotos();
          await this.saveOrder();
        });
      }

      grid.appendChild(item);
    });

    this.updateBulkToolbar();
  },

  async saveOrder() {
    if (!this.currentAlbumPath) return;
    try {
      const order = this.photoList.map(p => p.filename);
      await api.post(`/api/photo-order/${this.currentAlbumPath}`, { order });
      albums.currentPhotoOrder = order;

      // The gallery applies photoOrder only when the album sort is 'custom' —
      // switch it automatically so the drag result is what visitors see.
      const sortSelect = document.getElementById('album-sort');
      if (sortSelect.value !== 'custom') {
        await api.put(`/api/albums/${this.currentAlbumPath}`, { sort: 'custom' });
        sortSelect.value = 'custom';
        notifications.success('Order saved — album sort set to Custom');
      } else {
        notifications.success('Photo order saved');
      }
    } catch (error) {
      notifications.error('Failed to save order: ' + error.message);
    }
  },

  async bulkDelete() {
    const n = this.selected.size;
    if (n === 0) return;
    const confirmed = await modal.confirm(
      'Delete Photos',
      `Are you sure you want to delete ${n} photo(s)? This cannot be undone.`,
      'Delete'
    );
    if (!confirmed) return;

    try {
      const result = await api.post(`/api/photo-bulk/delete/${this.currentAlbumPath}`, {
        filenames: [...this.selected]
      });
      notifications.success(`Deleted ${result.deleted.length} photo(s)`);
      if (this.currentThumbnail && result.deleted.includes(this.currentThumbnail)) {
        this.currentThumbnail = null;
        document.getElementById('album-thumbnail').value = '';
      }
      this.setSelectionMode(false);
      await this.loadPhotos(this.currentAlbumPath, this.currentThumbnail);
      await albums.loadTree();
    } catch (error) {
      notifications.error('Failed to delete photos: ' + error.message);
    }
  },

  initMoveModal() {
    document.getElementById('move-photos-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const target = document.getElementById('move-target').value;
      if (!target) return;
      try {
        const result = await api.post(`/api/photo-bulk/move/${this.currentAlbumPath}`, {
          filenames: [...this.selected],
          target
        });
        let message = `Moved ${result.moved.length} photo(s) to ${target}`;
        if (result.renamed.length > 0) {
          message += ` (${result.renamed.length} renamed due to name conflicts)`;
        }
        notifications.success(message);
        modal.hide('move-photos-modal');
        this.setSelectionMode(false);
        await this.loadPhotos(this.currentAlbumPath, this.currentThumbnail);
        await albums.loadTree();
      } catch (error) {
        notifications.error('Failed to move photos: ' + error.message);
      }
    });
  },

  openMoveModal() {
    const select = document.getElementById('move-target');
    select.innerHTML = '';
    const flatten = (items, depth = 0) => {
      items.forEach(item => {
        if (item.path !== this.currentAlbumPath) {
          const option = document.createElement('option');
          option.value = item.path;
          option.textContent = `${'  '.repeat(depth)}${item.meta?.title || item.name}`;
          select.appendChild(option);
        }
        if (item.children) flatten(item.children, depth + 1);
      });
    };
    flatten(albums.tree);
    document.getElementById('move-count').textContent = this.selected.size;
    modal.show('move-photos-modal');
  },

  initPreviewModal() {
    // Close handled by generic modal close buttons; nothing else to wire here
  },

  async openPreview(photo) {
    const img = document.getElementById('preview-image');
    const title = document.getElementById('preview-title');
    const meta = document.getElementById('preview-meta');
    const exifEl = document.getElementById('preview-exif');

    title.textContent = photo.filename;
    meta.textContent = albums.formatFileSize(photo.size);
    exifEl.textContent = '';
    img.src = getAlbumImageUrl(this.currentAlbumPath, photo.filename);
    modal.show('photo-preview-modal');

    try {
      const { exif } = await api.get(`/api/photo-exif/${this.currentAlbumPath}/${encodeURIComponent(photo.filename)}`);
      const parts = [];
      const camera = [exif.Make, exif.Model].filter(Boolean).join(' ');
      if (camera) parts.push(camera);
      if (exif.LensModel) parts.push(exif.LensModel);
      if (exif.FocalLength) parts.push(`${exif.FocalLength}mm`);
      if (exif.FNumber) parts.push(`f/${exif.FNumber}`);
      if (exif.ExposureTime) {
        parts.push(exif.ExposureTime < 1 ? `1/${Math.round(1 / exif.ExposureTime)}s` : `${exif.ExposureTime}s`);
      }
      if (exif.ISO) parts.push(`ISO ${exif.ISO}`);
      if (exif.DateTimeOriginal) parts.push(new Date(exif.DateTimeOriginal).toLocaleString());
      exifEl.textContent = parts.length > 0 ? parts.join(' · ') : 'No EXIF data';
    } catch {
      exifEl.textContent = '';
    }
  },

  updateThumbnailSelect() {
    const select = document.getElementById('album-thumbnail');
    const previous = select.value || this.currentThumbnail || '';
    select.innerHTML = '<option value="">Auto (first photo)</option>';

    this.photoList.forEach(photo => {
      const option = document.createElement('option');
      option.value = photo.filename;
      option.textContent = photo.filename;
      if (photo.filename === previous) {
        option.selected = true;
      }
      select.appendChild(option);
    });
  },

  setAsCover(filename) {
    this.currentThumbnail = filename;
    document.getElementById('album-thumbnail').value = filename;
    this.renderPhotos();
    notifications.success(`"${filename}" set as cover photo. Don't forget to save!`);
  },

  async uploadPhotos(files) {
    if (!this.currentAlbumPath) {
      notifications.error('Please select an album first');
      return;
    }

    const uploadZone = document.getElementById('upload-zone');
    const originalText = uploadZone.innerHTML;
    const fileCount = files.length;
    uploadZone.innerHTML = `
      <p class="upload-status">Uploading ${fileCount} photo(s)… <span id="upload-percent">0%</span></p>
      <div class="progress-bar"><div class="progress-fill" id="upload-progress-fill"></div></div>
    `;

    try {
      const result = await api.uploadWithProgress(
        `/api/photos/${this.currentAlbumPath}`,
        Array.from(files),
        'photos',
        (percent) => {
          const fill = document.getElementById('upload-progress-fill');
          const label = document.getElementById('upload-percent');
          if (fill) fill.style.width = `${percent}%`;
          if (label) label.textContent = percent < 100 ? `${percent}%` : 'processing…';
        }
      );
      notifications.success(`Uploaded ${result.uploaded.length} photo(s)`);
      if (result.renamed && result.renamed.length > 0) {
        notifications.warning(
          `${result.renamed.length} file(s) already existed and were saved under a new name (e.g. ${result.renamed[0].to})`
        );
      }
      await this.loadPhotos(this.currentAlbumPath, this.currentThumbnail);
      await albums.loadTree(); // Refresh tree to update counts
    } catch (error) {
      notifications.error('Failed to upload photos: ' + error.message);
    } finally {
      uploadZone.innerHTML = originalText;
    }
  },

  async deletePhoto(filename) {
    if (!this.currentAlbumPath) return;

    try {
      await api.delete(`/api/photos/${encodeURIComponent(this.currentAlbumPath)}/file/${encodeURIComponent(filename)}`);
      notifications.success('Photo deleted');

      // Clear thumbnail if it was the deleted photo
      if (filename === this.currentThumbnail) {
        this.currentThumbnail = null;
        document.getElementById('album-thumbnail').value = '';
      }

      await this.loadPhotos(this.currentAlbumPath, this.currentThumbnail);
      await albums.loadTree();
    } catch (error) {
      notifications.error('Failed to delete photo: ' + error.message);
    }
  }
};
