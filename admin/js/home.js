// Home Content Module
const home = {
  introEditor: null,
  cardBodyEditor: null,
  cards: [],
  editingCardId: null,

  async init() {
    this.initIntroEditor();
    this.initEventListeners();
    this.initAssetHandlers();
  },

  initIntroEditor() {
    const textarea = document.getElementById('intro-body');
    if (textarea && typeof CodeMirror !== 'undefined') {
      this.introEditor = CodeMirror.fromTextArea(textarea, {
        mode: 'markdown',
        theme: 'monokai',
        lineWrapping: true,
        lineNumbers: false
      });
    }

    // Card body editor
    const cardTextarea = document.getElementById('card-body');
    if (cardTextarea && typeof CodeMirror !== 'undefined') {
      this.cardBodyEditor = CodeMirror.fromTextArea(cardTextarea, {
        mode: 'markdown',
        theme: 'monokai',
        lineWrapping: true,
        lineNumbers: false
      });
    }
  },

  initEventListeners() {
    // Save intro
    document.getElementById('save-intro-btn').addEventListener('click', () => {
      this.saveIntro();
    });

    // New card
    document.getElementById('new-card-btn').addEventListener('click', () => {
      this.openCardModal();
    });

    // Card form
    document.getElementById('card-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      await this.saveCard();
    });

    // Delete card
    document.getElementById('delete-card-btn').addEventListener('click', async () => {
      if (this.editingCardId) {
        const confirmed = await modal.confirm(
          'Delete Card',
          'Are you sure you want to delete this card?'
        );
        if (confirmed) {
          await this.deleteCard(this.editingCardId);
        }
      }
    });
  },

  async load() {
    await Promise.all([
      this.loadLandingBackground(),
      this.loadHeroImages(),
      this.loadIntro(),
      this.loadCards()
    ]);
  },

  async loadIntro() {
    try {
      const data = await api.get('/api/home/intro');
      if (this.introEditor) {
        this.introEditor.setValue(data.body || '');
      } else {
        document.getElementById('intro-body').value = data.body || '';
      }
    } catch (error) {
      notifications.error('Failed to load intro: ' + error.message);
    }
  },

  async saveIntro() {
    try {
      const body = this.introEditor ? this.introEditor.getValue() : document.getElementById('intro-body').value;
      await api.put('/api/home/intro', { body });
      notifications.success('Introduction saved');
    } catch (error) {
      notifications.error('Failed to save intro: ' + error.message);
    }
  },

  async loadCards() {
    try {
      this.cards = await api.get('/api/home/cards');
      this.renderCards();
    } catch (error) {
      notifications.error('Failed to load cards: ' + error.message);
    }
  },

  renderCards() {
    const container = document.getElementById('cards-list');
    container.innerHTML = '';

    if (this.cards.length === 0) {
      container.innerHTML = '<p style="color: var(--text-secondary);">No content cards yet. Add one!</p>';
      return;
    }

    this.cards.forEach(card => {
      const item = document.createElement('div');
      item.className = 'card-item';
      item.addEventListener('click', () => this.openCardModal(card));

      if (card.image) {
        const thumb = document.createElement('img');
        thumb.className = 'card-item-thumb';
        thumb.src = card.image;
        thumb.alt = card.title;
        // Handle broken images
        thumb.onerror = () => {
          thumb.style.display = 'none';
          const placeholder = document.createElement('div');
          placeholder.className = 'card-item-thumb';
          placeholder.style.display = 'flex';
          placeholder.style.alignItems = 'center';
          placeholder.style.justifyContent = 'center';
          placeholder.textContent = '🖼️';
          thumb.parentNode.insertBefore(placeholder, thumb);
        };
        item.appendChild(thumb);
      } else {
        const placeholder = document.createElement('div');
        placeholder.className = 'card-item-thumb';
        placeholder.style.display = 'flex';
        placeholder.style.alignItems = 'center';
        placeholder.style.justifyContent = 'center';
        placeholder.textContent = '📷';
        item.appendChild(placeholder);
      }

      const info = document.createElement('div');
      info.className = 'card-item-info';

      const title = document.createElement('div');
      title.className = 'card-item-title';
      title.textContent = card.title;
      info.appendChild(title);

      const meta = document.createElement('div');
      meta.className = 'card-item-meta';
      meta.textContent = `${card.imagePosition || 'left'} • ${card.link || 'no link'}`;
      info.appendChild(meta);

      item.appendChild(info);

      const order = document.createElement('div');
      order.className = 'card-item-order';
      order.textContent = card.order || '?';
      item.appendChild(order);

      container.appendChild(item);
    });
  },

  openCardModal(card = null) {
    const modalEl = document.getElementById('card-modal');
    const form = document.getElementById('card-form');
    const deleteBtn = document.getElementById('delete-card-btn');

    this.editingCardId = card?.id || null;

    document.getElementById('card-modal-title').textContent = card ? 'Edit Card' : 'New Card';
    deleteBtn.style.display = card ? 'inline-flex' : 'none';

    // Populate form
    form.querySelector('#card-id').value = card?.id || '';
    form.querySelector('#card-title').value = card?.title || '';
    form.querySelector('#card-image').value = card?.image || '';
    form.querySelector('#card-imagePosition').value = card?.imagePosition || 'left';
    form.querySelector('#card-link').value = card?.link || '';
    form.querySelector('#card-order').value = card?.order || (this.cards.length + 1);

    if (this.cardBodyEditor) {
      this.cardBodyEditor.setValue(card?.body || '');
      // Refresh CodeMirror after modal is shown
      setTimeout(() => this.cardBodyEditor.refresh(), 100);
    } else {
      form.querySelector('#card-body').value = card?.body || '';
    }

    // Update image preview
    this.updateCardImagePreview();

    modal.show('card-modal');
  },

  async saveCard() {
    const form = document.getElementById('card-form');
    const data = {
      id: form.querySelector('#card-id').value || undefined,
      title: form.querySelector('#card-title').value,
      image: form.querySelector('#card-image').value || undefined,
      imagePosition: form.querySelector('#card-imagePosition').value,
      link: form.querySelector('#card-link').value || undefined,
      order: parseInt(form.querySelector('#card-order').value) || 1,
      body: this.cardBodyEditor ? this.cardBodyEditor.getValue() : form.querySelector('#card-body').value
    };

    try {
      if (this.editingCardId) {
        await api.put(`/api/home/cards/${this.editingCardId}`, data);
        notifications.success('Card updated');
      } else {
        await api.post('/api/home/cards', data);
        notifications.success('Card created');
      }

      modal.hide('card-modal');
      await this.loadCards();
    } catch (error) {
      notifications.error('Failed to save card: ' + error.message);
    }
  },

  async deleteCard(id) {
    try {
      await api.delete(`/api/home/cards/${id}`);
      notifications.success('Card deleted');
      modal.hide('card-modal');
      await this.loadCards();
    } catch (error) {
      notifications.error('Failed to delete card: ' + error.message);
    }
  },

  // Asset handling for landing, hero images and card images
  initAssetHandlers() {
    // Landing background upload
    const landingUpload = document.getElementById('landing-upload');
    if (landingUpload) {
      landingUpload.addEventListener('change', async (e) => {
        if (e.target.files.length > 0) {
          await this.uploadLandingBackground(e.target.files[0]);
          e.target.value = '';
        }
      });
    }

    // Hero image upload
    const heroUpload = document.getElementById('home-hero-upload');
    if (heroUpload) {
      heroUpload.addEventListener('change', async (e) => {
        if (e.target.files.length > 0) {
          await this.uploadHeroImage(e.target.files[0]);
          e.target.value = '';
        }
      });
    }

    // Card image upload in modal
    const cardImageUpload = document.getElementById('card-image-upload-modal');
    if (cardImageUpload) {
      cardImageUpload.addEventListener('change', async (e) => {
        if (e.target.files.length > 0) {
          await this.uploadCardImage(e.target.files[0]);
          e.target.value = '';
        }
      });
    }

    // Update preview when image URL changes
    const cardImageInput = document.getElementById('card-image');
    if (cardImageInput) {
      cardImageInput.addEventListener('input', () => this.updateCardImagePreview());
    }
  },

  loadLandingBackground() {
    const preview = document.getElementById('landing-preview');
    if (preview) {
      // Add cache-busting timestamp
      preview.innerHTML = `<img src="/images/landing-bg.jpg?t=${Date.now()}" alt="Landing background">`;
    }
  },

  async uploadLandingBackground(file) {
    try {
      await api.upload('/api/assets/landing', file, 'image');
      notifications.success('Landing background updated');
      this.loadLandingBackground();
    } catch (error) {
      notifications.error('Failed to upload landing background: ' + error.message);
    }
  },

  async loadHeroImages() {
    try {
      const images = await api.get('/api/assets/hero');
      const container = document.getElementById('home-hero-images');
      if (!container) {
        return;
      }

      container.innerHTML = '';

      if (images.length === 0) {
        container.innerHTML = '<p style="color: var(--text-secondary); grid-column: 1/-1;">No hero images yet. Upload some to show in the slider.</p>';
        return;
      }

      images.forEach(img => {
        const item = document.createElement('div');
        item.className = 'image-item';

        const image = document.createElement('img');
        image.src = img.url;
        image.alt = img.filename;
        image.onerror = () => {
          image.style.display = 'none';
        };
        item.appendChild(image);

        const deleteBtn = document.createElement('button');
        deleteBtn.textContent = '×';
        deleteBtn.addEventListener('click', async (e) => {
          e.stopPropagation();
          const confirmed = await modal.confirm('Delete Image', `Delete "${img.filename}"?`);
          if (confirmed) {
            await this.deleteHeroImage(img.filename);
          }
        });
        item.appendChild(deleteBtn);

        container.appendChild(item);
      });
    } catch (error) {
      notifications.error('Failed to load hero images: ' + error.message);
    }
  },

  async uploadHeroImage(file) {
    try {
      await api.upload('/api/assets/hero', file, 'image');
      notifications.success('Hero image uploaded');
      await this.loadHeroImages();
    } catch (error) {
      notifications.error('Failed to upload hero image: ' + error.message);
    }
  },

  async deleteHeroImage(filename) {
    try {
      await api.delete(`/api/assets/hero/${filename}`);
      notifications.success('Hero image deleted');
      await this.loadHeroImages();
    } catch (error) {
      notifications.error('Failed to delete hero image: ' + error.message);
    }
  },

  async uploadCardImage(file) {
    try {
      const result = await api.upload('/api/assets/cards', file, 'image');
      notifications.success('Card image uploaded');

      // Set the URL in the input field
      const imageInput = document.getElementById('card-image');
      if (imageInput && result.url) {
        imageInput.value = result.url;
        this.updateCardImagePreview();
      }
    } catch (error) {
      notifications.error('Failed to upload card image: ' + error.message);
    }
  },

  updateCardImagePreview() {
    const imageInput = document.getElementById('card-image');
    const preview = document.getElementById('card-image-preview');
    if (!imageInput || !preview) return;

    const url = imageInput.value.trim();
    if (url) {
      preview.innerHTML = `<img src="${url}" alt="Preview" style="max-width: 200px; max-height: 100px; border-radius: 4px; margin-top: 0.5rem;">`;
    } else {
      preview.innerHTML = '';
    }
  }
};
