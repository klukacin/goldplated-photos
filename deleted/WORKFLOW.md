# Photo Gallery Workflow

This guide explains how to manage your content locally and deploy it to your production server (`k.lukacin.com`).

## 1. How It Works
*   **Source of Truth:** Your local computer is the master copy. All photos, albums, and text live in `src/content/albums/` and `src/content/home/`.
*   **Deployment:** When you run `npm run deploy`, the system copies your *source* files to the server. It does not matter that they aren't in the `dist/` folder; the script handles the sync separately.

## 2. Managing Content (Locally)

You have two options for managing content.

### Option A: Using the Admin Interface (Recommended)
This is the easiest way to create albums, upload photos, and organize the home page.

1.  **Start the Admin Panel:**
    ```bash
    npm run admin
    ```
2.  **Open in Browser:** Go to `http://localhost:4444`.
3.  **Perform Actions:**
    *   **Create Albums:** Click "New Album" to create folders.
    *   **Upload Photos:** Drag and drop photos into albums.
    *   **Edit Home Page:** Go to the "Home" tab to reorder cards or change the intro text.
    *   **Passwords:** Set passwords for private albums directly in the settings.

### Option B: Manual File Management
Since the gallery is file-system based, you can just use Finder/Explorer.

1.  **Go to the Content Directory:**
    *   Open `src/content/albums/` in your file explorer.
2.  **Create Folders:**
    *   Create a folder like `2025/Vacations/Paris`.
    *   *Note:* You must add an `index.md` file to new folders for them to appear (copy one from another folder or use the Admin panel to generate it).
3.  **Add Photos:**
    *   Paste `.jpg` or `.png` images directly into the folder.

## 3. Previewing Changes
Before deploying, you can see exactly how your site will look.

1.  **Start the Dev Server:**
    ```bash
    npm run dev
    ```
2.  **View Site:** Open `http://localhost:4321`.

## 4. Deploying to Production
When you are happy with your changes, push them to the live server.

1.  **Run the Deploy Script:**
    ```bash
    npm run deploy
    ```
2.  **What this does:**
    *   Rebuilds the website code (`npm run build`).
    *   Syncs all your photos from `src/content/albums/` to the server.
    *   Syncs your home page configurations.
    *   Restarts the live server.

## 5. Troubleshooting
*   **"Photos aren't showing up":** Ensure you didn't just put them in `dist/`. They must be in `src/content/albums`.
*   **"Permission denied":** The deploy script uses your configured SSH key (`~/.ssh/id_rsa-ws`). If you change computers, you'll need to set up SSH keys again.
*   **"Server Error 500" or "Password Error":** If the live site crashes or passwords fail:
    1.  Check logs: `ssh klukacincom@k.lukacin.com "tail -n 20 /home/klukacincom/public_html/server.log"`
    2.  Restart manually:
        ```bash
        ssh klukacincom@k.lukacin.com "cd public_html && pkill -f entry.mjs; nohup HOST=127.0.0.1 PORT=4321 node server/entry.mjs > server.log 2>&1 &"
        ```
