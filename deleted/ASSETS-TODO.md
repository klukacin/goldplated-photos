# Assets To Download and Replace

The site is now fully functional with placeholder files, but you need to replace them with real images and sounds.

## ✅ Site is Working!

All routes are functional:
- `/` - Landing page (shutter button)
- `/home` - Digital home with content blocks
- `/photos` - Photo gallery
- `/photos/*` - All album pages

## 📥 Assets to Download

### 1. Landing Page Background Image

**Location:** `public/images/landing-bg.jpg`

**How to get it:**
1. Visit https://k.lukac.in/
2. Open browser DevTools (F12 or right-click → Inspect)
3. Go to the **Network** tab
4. Refresh the page (Ctrl+R or Cmd+R)
5. Look for image files being loaded (filter by "Img" type)
6. Find the background image (likely a large .jpg file)
7. Right-click on it in the Network tab → "Open in new tab"
8. Right-click the image → "Save image as..."
9. Save as `landing-bg.jpg` in `public/images/`

**Alternative:**
- Right-click on the background → Inspect Element
- Look for `background-image: url(...)` in the Styles panel
- Copy the URL and download it directly

---

### 2. Camera Shutter Sound

**Location:** `public/sounds/shutter.mp3`

**Where to find:**
1. **Freesound.org** (recommended):
   - Go to https://freesound.org/
   - Search for "camera shutter"
   - Filter by license: "Creative Commons 0"
   - Download a short (0.2-0.5 second) shutter sound
   - Save as `shutter.mp3` in `public/sounds/`

2. **Other sources:**
   - https://mixkit.co/free-sound-effects/camera/
   - https://www.zapsplat.com/ (search "camera shutter")

**Requirements:**
- Format: MP3
- Duration: 0.2-0.5 seconds (short click sound)
- License: Free to use

---

### 3. Hero Slider Images (3 images)

**Location:** `public/home/hero/`

**Files needed:**
- `image1.jpg`
- `image2.jpg`
- `image3.jpg`

**How to get them:**
1. Visit https://k.lukac.in/digital-home
2. Look for the hero/banner images (wide, prominent images)
3. Right-click on each image → "Save image as..."
4. Rename them to `image1.jpg`, `image2.jpg`, `image3.jpg`
5. Place in `public/home/hero/`

**Requirements:**
- Wide format (16:9 or similar aspect ratio)
- At least 1920px wide for best quality
- High resolution

---

### 4. Content Card Images (6 images)

**Location:** `public/home/cards/`

**Files needed:**
1. `weddings.jpg` - For "Weddings" section
2. `people.jpg` - For "People" section
3. `events.jpg` - For "Public Events" section
4. `street.jpg` - For "Street, Travels and Architecture" section
5. `markusevec.jpg` - For "Markuševec" section
6. `friends.jpg` - For "Friends and Family Only" section

**How to get them:**
1. Visit https://k.lukac.in/digital-home
2. Scroll through the page and find images for each section
3. Right-click on each section's image → "Save image as..."
4. Rename to match the filenames above
5. Place all in `public/home/cards/`

**Requirements:**
- Landscape/rectangular format
- At least 800px wide
- Good quality for display

---

## 📝 Current Status

✅ **Implemented and Working:**
- Landing page with shutter button and sound
- Digital home page with hero slider
- Digital home content blocks (intro + 6 cards)
- Photo gallery moved to `/photos/*`
- All album functionality preserved
- Breadcrumbs updated
- Cover photos working
- Password protection working
- All previous features intact

⚠️ **Using Placeholders:**
- All images are text placeholder files
- Shutter sound is a text placeholder file

🎯 **Next Steps:**
1. Download the assets listed above
2. Replace the placeholder files
3. Run `npm run dev` to test
4. Customize content in `src/content/home/` markdown files if needed

---

## 🔧 Testing After Replacing Assets

After you've downloaded and replaced the assets:

```bash
# Build the site
npm run build

# Start the preview server
npm run preview

# Or run in development mode
npm run dev
```

Then visit:
- http://localhost:4321/ (landing page)
- http://localhost:4321/home (digital home)
- http://localhost:4321/photos (photo gallery)

---

## ✏️ Customizing Content

All text content for the `/home` page can be edited in these markdown files:

- `src/content/home/intro.md` - Introduction paragraph
- `src/content/home/cards/weddings.md` - Weddings section text
- `src/content/home/cards/people.md` - People section text
- `src/content/home/cards/events.md` - Public Events section text
- `src/content/home/cards/street.md` - Street/Travel section text
- `src/content/home/cards/markusevec.md` - Markuševec section text
- `src/content/home/cards/friends.md` - Friends & Family section text

You can:
- Edit the text in each file
- Change the `imagePosition` (left or right)
- Update the `link` to point to different album paths
- Adjust the `order` to change the sequence

---

## 🎨 Note About Images

The current text placeholder files will not display as images in the browser. They're just placeholders so the build doesn't fail. Once you replace them with actual JPG files, everything will look correct!
