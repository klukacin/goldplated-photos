// Photos Module
const photos = {
  currentAlbumPath: null,
  photoList: [],
  currentThumbnail: null,

  init() {
    this.initEventListeners();
    this.initDragDrop();
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

  async loadPhotos(albumPath, currentThumbnail = null) {
    this.currentAlbumPath = albumPath;
    this.currentThumbnail = currentThumbnail;

    try {
      this.photoList = await api.get(`/api/photos/${albumPath}`);
      this.renderPhotos();
      this.updateThumbnailSelect();
      // Update photo count badge
      document.getElementById('photo-count-badge').textContent = this.photoList.length;
    } catch (error) {
      notifications.error('Failed to load photos: ' + error.message);
    }
  },

  renderPhotos() {
    const grid = document.getElementById('photo-grid');
    grid.innerHTML = '';

    if (this.photoList.length === 0) {
      grid.innerHTML = '<p style="color: var(--text-secondary); grid-column: 1/-1;">No photos in this album yet.</p>';
      return;
    }

    this.photoList.forEach(photo => {
      const item = document.createElement('div');
      item.className = 'photo-item';
      if (photo.filename === this.currentThumbnail) {
        item.classList.add('is-cover');
      }

      const img = document.createElement('img');
      // Use small thumbnail for faster loading in admin
      img.src = `http://localhost:4321/api/thumbnail?path=${encodeURIComponent(this.currentAlbumPath + '/' + photo.filename)}&size=small`;
      img.alt = photo.filename;
      img.loading = 'lazy';
      // Fallback to original if thumbnail fails
      img.onerror = () => {
        img.src = getAlbumImageUrl(this.currentAlbumPath, photo.filename);
      };
      item.appendChild(img);

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
          `Are you sure you want to delete "${photo.filename}"?`
        );
        if (confirmed) {
          await this.deletePhoto(photo.filename);
        }
      });
      overlay.appendChild(deleteBtn);

      item.appendChild(overlay);
      grid.appendChild(item);
    });
  },

  updateThumbnailSelect() {
    const select = document.getElementById('album-thumbnail');
    select.innerHTML = '<option value="">Auto (first photo)</option>';

    this.photoList.forEach(photo => {
      const option = document.createElement('option');
      option.value = photo.filename;
      option.textContent = photo.filename;
      if (photo.filename === this.currentThumbnail) {
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
    uploadZone.innerHTML = `<p>Uploading ${files.length} photo(s)...</p>`;

    try {
      const result = await api.upload(`/api/photos/${this.currentAlbumPath}`, Array.from(files));
      notifications.success(`Uploaded ${result.uploaded.length} photo(s)`);
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
