// Backuper landing page: fills the download buttons and the version list from GitHub Releases,
// and opens screenshots in a lightbox. Everything else on the page is static HTML.
(() => {
  "use strict";

  const REPO = "Star69995/backuper";
  const RELEASES_PAGE = `https://github.com/${REPO}/releases`;
  const API = `https://api.github.com/repos/${REPO}/releases?per_page=100`;
  const CACHE_KEY = "backuper-releases-v1";
  const CACHE_MS = 10 * 60 * 1000;
  const SHOWN_AT_FIRST = 5;

  const $ = (sel, root = document) => root.querySelector(sel);

  const esc = (s) =>
    String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

  const icon = (name) => `<svg class="ic" aria-hidden="true"><use href="#i-${name}"/></svg>`;

  const formatSize = (bytes) => {
    if (!bytes) return "";
    const mb = bytes / (1024 * 1024);
    return mb >= 1 ? `${mb.toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1024))} KB`;
  };

  const formatDate = (iso) => {
    try {
      return new Date(iso).toLocaleDateString("he-IL", { day: "numeric", month: "long", year: "numeric" });
    } catch {
      return "";
    }
  };

  const versionOf = (rel) => String(rel.tag_name || rel.name || "").replace(/^v/i, "");

  /** The installer asset of a release: a *-setup.exe first, then any .exe / .msi. */
  const installerOf = (rel) => {
    const assets = (rel.assets || []).filter((a) => /\.(exe|msi)$/i.test(a.name));
    return assets.find((a) => /setup\.exe$/i.test(a.name)) || assets[0] || null;
  };

  // --- Data ----------------------------------------------------------------

  const readCache = () => {
    try {
      const c = JSON.parse(sessionStorage.getItem(CACHE_KEY) || "null");
      return c && Date.now() - c.t < CACHE_MS ? c.data : null;
    } catch {
      return null;
    }
  };

  const writeCache = (data) => {
    try {
      sessionStorage.setItem(CACHE_KEY, JSON.stringify({ t: Date.now(), data }));
    } catch {
      /* storage unavailable - fine, just no cache */
    }
  };

  async function loadReleases() {
    const cached = readCache();
    if (cached) return cached;
    // The html+json media type adds `body_html`: the release notes, already rendered and sanitized by GitHub.
    const res = await fetch(API, { headers: { Accept: "application/vnd.github.html+json" } });
    if (!res.ok) throw new Error(`GitHub API ${res.status}`);
    const data = (await res.json())
      .filter((r) => !r.draft)
      .map((r) => ({
        tag_name: r.tag_name,
        name: r.name,
        html_url: r.html_url,
        prerelease: r.prerelease,
        published_at: r.published_at,
        body_html: r.body_html || "",
        assets: (r.assets || []).map((a) => ({
          name: a.name,
          size: a.size,
          browser_download_url: a.browser_download_url,
          digest: a.digest || null,
        })),
      }));
    writeCache(data);
    return data;
  }

  // --- Rendering -------------------------------------------------------------

  function renderHero(latest) {
    const meta = $("#hero-meta");
    const asset = latest && installerOf(latest);
    if (!asset) {
      if (!latest) {
        meta.textContent = "הגרסה הראשונה תפורסם בקרוב";
        document.querySelectorAll("[data-latest-link]").forEach((a) => (a.href = "#versions"));
      }
      return;
    }
    document.querySelectorAll("[data-latest-link]").forEach((a) => {
      a.href = asset.browser_download_url;
    });
    meta.innerHTML = `גרסה <bdi dir="ltr">${esc(versionOf(latest))}</bdi> · <bdi dir="ltr">${esc(formatSize(asset.size))}</bdi> · Windows 10 ו-11`;
  }

  function releaseCard(rel, isLatest) {
    const asset = installerOf(rel);
    const version = versionOf(rel);
    const badges =
      (isLatest ? `<span class="badge">האחרונה</span>` : "") +
      (rel.prerelease ? `<span class="badge badge-pre">גרסת בדיקה</span>` : "");

    const download = asset
      ? `<div class="release-dl">
           <a class="btn ${isLatest ? "btn-primary" : "btn-secondary"} btn-md" href="${esc(asset.browser_download_url)}">
             ${icon("download")}<span>הורדה</span>
           </a>
           <small><bdi dir="ltr">${esc(formatSize(asset.size))}</bdi></small>
         </div>`
      : `<div class="release-dl">
           <a class="btn btn-secondary btn-md" href="${esc(rel.html_url)}">${icon("external")}<span>לדף הגרסה</span></a>
         </div>`;

    const notes = rel.body_html.trim()
      ? `<details class="disclosure"${isLatest ? " open" : ""}>
           <summary>${icon("chevron").replace('class="ic"', 'class="ic chev"')}מה חדש בגרסה הזו</summary>
           <div class="disclosure-body notes" dir="auto">${rel.body_html}</div>
         </details>`
      : "";

    const sha = asset && /^sha256:/i.test(asset.digest || "") ? asset.digest.slice(7) : "";
    const hash = sha
      ? `<div class="hash">
           טביעת אצבע (SHA-256) של קובץ ההתקנה:
           <div class="hash-row"><code dir="ltr">${esc(sha)}</code><button type="button" class="copy-btn" data-copy="${esc(sha)}">העתקה</button></div>
         </div>`
      : "";

    return `<article class="release${isLatest ? " is-latest" : ""}">
      <div class="release-head">
        <div class="release-title">
          <h3><bdi dir="ltr">${esc(rel.name && rel.name !== rel.tag_name ? rel.name : version)}</bdi>${badges}</h3>
          <div class="when">פורסמה ב-${esc(formatDate(rel.published_at))}</div>
        </div>
        ${download}
      </div>
      ${notes}
      ${hash}
    </article>`;
  }

  function renderList(releases, latest) {
    const box = $("#releases");
    if (!releases.length) {
      box.innerHTML = `<div class="notice">
        <p>עדיין לא פורסמה גרסה להורדה. היא תופיע כאן ברגע שתעלה.</p>
        <a href="${RELEASES_PAGE}">${icon("github")} מעקב אחרי הפרויקט ב-GitHub</a>
      </div>`;
      return;
    }
    const cards = releases.map((r) => releaseCard(r, r === latest));
    const rest = cards.length - SHOWN_AT_FIRST;
    box.innerHTML =
      cards.slice(0, SHOWN_AT_FIRST).join("") +
      (rest > 0
        ? `<div class="more-hidden" hidden>${cards.slice(SHOWN_AT_FIRST).join("")}</div>
           <div class="more-wrap"><button type="button" class="btn btn-secondary btn-md" id="show-more">הצגת ${rest} גרסאות קודמות</button></div>`
        : "") +
      `<p class="more-wrap"><a href="${RELEASES_PAGE}">${icon("external")} כל הגרסאות גם בדף ב-GitHub</a></p>`;

    const more = $("#show-more");
    if (more) {
      more.addEventListener("click", () => {
        const hidden = $(".more-hidden", box);
        hidden.hidden = false;
        hidden.style.display = "contents";
        more.parentElement.remove();
      });
    }
  }

  function renderError() {
    $("#releases").innerHTML = `<div class="notice">
      <p>לא הצלחנו לטעון את רשימת הגרסאות כרגע. אפשר לנסות שוב בעוד רגע, או להוריד ישירות מ-GitHub:</p>
      <a class="btn btn-primary btn-md" href="${RELEASES_PAGE}/latest">${icon("download")}<span>לגרסה האחרונה ב-GitHub</span></a>
    </div>`;
  }

  loadReleases()
    .then((releases) => {
      const latest = releases.find((r) => !r.prerelease) || releases[0] || null;
      renderHero(latest);
      renderList(releases, latest);
    })
    .catch(renderError);

  // Copy a SHA-256 to the clipboard.
  document.addEventListener("click", async (e) => {
    const btn = e.target.closest("[data-copy]");
    if (!btn) return;
    try {
      await navigator.clipboard.writeText(btn.dataset.copy);
      btn.textContent = "הועתק";
      setTimeout(() => (btn.textContent = "העתקה"), 1600);
    } catch {
      /* clipboard blocked - the hash text is still selectable */
    }
  });

  // --- Lightbox --------------------------------------------------------------

  const box = $("#lightbox");
  if (box && typeof box.showModal === "function") {
    const img = $("img", box);
    const caption = $(".lightbox-caption", box);
    document.querySelectorAll(".shot-btn").forEach((btn) => {
      btn.addEventListener("click", () => {
        const thumb = $("img", btn);
        img.src = btn.dataset.full;
        img.alt = thumb.alt;
        caption.textContent = $("figcaption strong", btn.closest("figure"))?.textContent || "";
        box.showModal();
      });
    });
    $(".lightbox-close", box).addEventListener("click", () => box.close());
    // Click outside the image closes.
    box.addEventListener("click", (e) => {
      if (e.target === box) box.close();
    });
  }
})();
