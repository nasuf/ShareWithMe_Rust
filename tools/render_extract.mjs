import { chromium } from 'playwright-core';
import { existsSync } from 'node:fs';

// Usage: node render_extract.mjs <url> [--snapshot <output.jpg>]
//   --snapshot also writes a JPEG screenshot of the article container so the
//   backend can keep a visual archive that survives the source going offline.
const targetUrl = process.argv[2];
const snapshotFlag = process.argv.indexOf('--snapshot');
const snapshotPath = snapshotFlag > 0 ? process.argv[snapshotFlag + 1] : '';

const MAX_CONTENT_HTML = 2_000_000;
// Pixel cap: iOS cannot upload taller textures, and decoding cost grows fast.
const MAX_SNAPSHOT_HEIGHT = 16000;
const SNAPSHOT_SCALE = 2;

function cleanText(value, max = 14000) {
  return String(value || '')
    .replace(/\u00a0/g, ' ')
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .replace(/[ \t]{2,}/g, ' ')
    .trim()
    .slice(0, max);
}

function firstNonEmpty(...values) {
  return values.map((value) => cleanText(value, 4000)).find(Boolean) || '';
}

function chromeExecutable() {
  const candidates = [
    process.env.CHROME_PATH,
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Chromium.app/Contents/MacOS/Chromium',
    '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
    '/usr/bin/chromium-browser'
  ].filter(Boolean);
  return candidates.find((candidate) => existsSync(candidate));
}

async function extractPage(url) {
  const executablePath = chromeExecutable();
  if (!executablePath) {
    throw new Error('Chrome/Chromium executable not found. Set CHROME_PATH to enable rendered extraction.');
  }

  const browser = await chromium.launch({
    executablePath,
    headless: true,
    args: ['--disable-dev-shm-usage']
  });
  try {
    // Xiaohongshu sends desktop browsers to its login wall but serves the full
    // note page to mobile Safari, so those hosts render as a phone.
    const mobile = isXiaohongshuUrl(url);
    const page = await browser.newPage({
      userAgent: mobile
        ? 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1'
        : 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36',
      viewport: mobile ? { width: 430, height: 932 } : { width: 1365, height: 900 },
      isMobile: mobile,
      hasTouch: mobile,
      // Archive snapshots render at 2x so text stays sharp on retina phones.
      deviceScaleFactor: snapshotPath ? SNAPSHOT_SCALE : 1,
      locale: 'zh-CN'
    });
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
    await page.waitForTimeout(6000);
    // Walk the page once so lazy-loaded images swap their placeholder `src`
    // for the real one (needed for both the HTML blocks and the screenshot).
    await autoScroll(page);

    const result = await page.evaluate(({ maxContentHtml }) => {
      function clean(value, max = 14000) {
        return String(value || '')
          .replace(/\u00a0/g, ' ')
          .replace(/[ \t]+\n/g, '\n')
          .replace(/\n{3,}/g, '\n\n')
          .replace(/[ \t]{2,}/g, ' ')
          .trim()
          .slice(0, max);
      }

      function meta(selector) {
        return document.querySelector(selector)?.getAttribute('content') || '';
      }

      function textOf(selector) {
        return clean(document.querySelector(selector)?.innerText || '');
      }

      function bestDomText() {
        const selectors = [
          'article',
          '#js_content',
          '.rich_media_content',
          '.syl-page-article',
          '.article-content',
          '.article-detail-container',
          'main',
          '[class*=note-content]',
          '[class*=content]',
          '[class*=detail]'
        ];
        const candidates = selectors
          .flatMap((selector) => Array.from(document.querySelectorAll(selector)).slice(0, 8))
          .map((element) => clean(element.innerText || ''))
          .filter((text) => text.length >= 80)
          .sort((a, b) => b.length - a.length);

        const articleText = textOf('article') || textOf('.syl-page-article');
        return articleText.length >= 80 ? articleText : candidates[0] || clean(document.body?.innerText || '');
      }

      // The article container as HTML, so the backend can rebuild headings,
      // images and captions instead of flattening everything to innerText.
      function contentHtml() {
        const selectors = [
          '#js_content',
          '.syl-page-article',
          'article',
          '.rich_media_content',
          '.article-content',
          '[class*=note-content]',
          'main'
        ];
        for (const selector of selectors) {
          const element = document.querySelector(selector);
          if (element && clean(element.innerText || '').length >= 80) {
            const html = element.outerHTML || '';
            return html.length <= maxContentHtml ? html : '';
          }
        }
        return '';
      }

      function jsonLd() {
        for (const node of document.querySelectorAll('script[type="application/ld+json"]')) {
          try {
            const parsed = JSON.parse(node.textContent || '{}');
            const item = Array.isArray(parsed) ? parsed[0] : parsed;
            if (item && typeof item === 'object') return item;
          } catch (_) {
            continue;
          }
        }
        return {};
      }

      function xhsNoteFromState() {
        const state = window.__INITIAL_STATE__;
        const detailMap = state?.note?.noteDetailMap;
        if (detailMap && typeof detailMap === 'object') {
          for (const detail of Object.values(detailMap)) {
            const note = detail?.note;
            if (note && (note.title || note.desc)) return note;
          }
        }
        // Mobile page shape.
        const mobileNote = state?.noteData?.data?.noteData;
        if (mobileNote && (mobileNote.title || mobileNote.desc)) return mobileNote;
        return null;
      }

      function wechatArticle() {
        const body = document.querySelector('#js_content');
        if (!body) return null;
        const title = clean(
          document.querySelector('#activity-name')?.innerText ||
            meta('meta[property="og:title"]') ||
            document.title,
          4000
        );
        const author = clean(
          document.querySelector('#js_name')?.innerText ||
            document.querySelector('.profile_nickname')?.innerText ||
            '',
          4000
        );
        return {
          ok: true,
          finalUrl: location.href,
          title,
          description: clean(meta('meta[property="og:description"]') || meta('meta[name="description"]'), 4000),
          author,
          imageUrl: meta('meta[property="og:image"]') || '',
          contentText: clean(body.innerText || ''),
          contentHtml: contentHtml(),
          extractor: 'wechat-render'
        };
      }

      const ld = jsonLd();
      const wechat = wechatArticle();
      if (wechat && wechat.contentText.length >= 80) {
        return wechat;
      }
      const xhsNote = xhsNoteFromState();
      if (xhsNote) {
        const firstImage = xhsNote.imageList?.[0] || {};
        const image = firstImage.urlDefault || firstImage.url || firstImage.urlPre || '';
        return {
          ok: true,
          finalUrl: location.href,
          title: clean(xhsNote.title || document.title, 4000),
          description: clean(xhsNote.desc || '', 4000),
          author: clean(xhsNote.user?.nickname || xhsNote.user?.nickName || '', 4000),
          imageUrl: image,
          contentText: clean([xhsNote.title, xhsNote.desc].filter(Boolean).join('\n\n')),
          extractor: 'xhs-initial-state'
        };
      }

      const title = clean(
        meta('meta[property="og:title"]') ||
          meta('meta[name="twitter:title"]') ||
          ld.headline ||
          document.title,
        4000
      );
      const description = clean(
        meta('meta[property="og:description"]') ||
          meta('meta[name="description"]') ||
          meta('meta[name="twitter:description"]') ||
          ld.description,
        4000
      );
      const author =
        typeof ld.author === 'string'
          ? ld.author
          : ld.author?.name || meta('meta[name="author"]') || meta('meta[property="article:author"]') || '';
      const image =
        meta('meta[property="og:image"]') ||
        meta('meta[name="twitter:image"]') ||
        (Array.isArray(ld.image) ? ld.image[0] : ld.image || '');

      return {
        ok: true,
        finalUrl: location.href,
        title,
        description,
        author: clean(author, 4000),
        imageUrl: image,
        contentText: bestDomText(),
        contentHtml: contentHtml(),
        extractor: 'browser-render'
      };
    }, { maxContentHtml: MAX_CONTENT_HTML });

    // Ended on a login / captcha wall instead of the content: report failure
    // so the backend neither saves the wall as content nor archives it.
    if (isWallUrl(page.url())) {
      return { ok: false, error: `blocked by login/captcha wall: ${page.url()}` };
    }

    if (snapshotPath) {
      try {
        result.snapshot = await captureSnapshot(page, snapshotPath);
      } catch (error) {
        result.snapshotError = error instanceof Error ? error.message : String(error);
      }
    }
    return result;
  } finally {
    await browser.close();
  }
}

function isXiaohongshuUrl(url) {
  try {
    const host = new URL(url).hostname.replace(/^www\./, '');
    return ['xiaohongshu.com', 'xhslink.com', 'xhslink.cn', 'rednote.com'].some(
      (domain) => host === domain || host.endsWith(`.${domain}`)
    );
  } catch {
    return false;
  }
}

function isWallUrl(url) {
  try {
    const parsed = new URL(url);
    return (
      isXiaohongshuUrl(url) &&
      (parsed.pathname.startsWith('/login') ||
        parsed.pathname.startsWith('/website-login') ||
        parsed.pathname.startsWith('/404'))
    );
  } catch {
    return false;
  }
}

async function autoScroll(page) {
  await page.evaluate(async () => {
    const step = Math.max(400, Math.floor(window.innerHeight * 0.8));
    for (let index = 0; index < 60; index += 1) {
      const before = window.scrollY;
      window.scrollBy(0, step);
      await new Promise((resolve) => setTimeout(resolve, 250));
      if (window.scrollY === before) break;
    }
    window.scrollTo(0, 0);
  });
  // Give the images that just entered the viewport a moment to decode.
  await page
    .waitForFunction(
      () => Array.from(document.images).every((image) => image.complete),
      null,
      { timeout: 6000 }
    )
    .catch(() => {});
}

// Screenshot the article container (title + body) rather than the whole page,
// which drops navigation, sidebars and recommendation feeds.
async function captureSnapshot(page, outputPath) {
  const measure = () =>
    page.evaluate(() => {
      const selectors = [
        '.article-content',
        '#img-content',
        '.note-container',
        '#js_article',
        'article',
        'main'
      ];
      for (const selector of selectors) {
        const element = document.querySelector(selector);
        if (!element) continue;
        const rect = element.getBoundingClientRect();
        if (rect.width >= 280 && rect.height >= 300) {
          return {
            x: Math.max(0, rect.left + window.scrollX),
            y: Math.max(0, rect.top + window.scrollY),
            width: rect.width,
            height: rect.height
          };
        }
      }
      return {
        x: 0,
        y: 0,
        width: document.documentElement.clientWidth,
        height: document.documentElement.scrollHeight
      };
    });

  // Fixed/sticky bars would otherwise be painted over the article.
  await page.evaluate(() => {
    for (const element of document.querySelectorAll('body *')) {
      const position = getComputedStyle(element).position;
      if (position === 'fixed' || position === 'sticky') {
        element.style.setProperty('display', 'none', 'important');
      }
    }
  });

  // Keep 2x when the whole article fits under the pixel cap; a longer article
  // drops to 1x so it is still archived in full rather than cut off.
  const box = await measure();
  const fullHeight = Math.ceil(box.height);
  const scale = fullHeight * SNAPSHOT_SCALE <= MAX_SNAPSHOT_HEIGHT ? SNAPSHOT_SCALE : 1;
  const height = Math.min(fullHeight, Math.floor(MAX_SNAPSHOT_HEIGHT / scale));

  // Size the viewport to the article and scroll to it, instead of `fullPage`:
  // fullPage resizes to the entire document, which on long pages (tens of
  // thousands of px) takes longer than the screenshot timeout.
  const viewport = page.viewportSize();
  await page.setViewportSize({ width: viewport.width, height });
  const settled = await measure();
  const top = await page.evaluate((y) => {
    window.scrollTo(0, y);
    return y - window.scrollY;
  }, settled.y);
  await page.waitForTimeout(400);

  const clip = {
    x: Math.floor(settled.x),
    y: Math.max(0, Math.floor(top)),
    width: Math.ceil(settled.width),
    height
  };
  await page.screenshot({
    path: outputPath,
    type: 'jpeg',
    quality: 72,
    clip,
    scale: scale === 1 ? 'css' : 'device',
    timeout: 45000
  });
  // Report pixel dimensions: the app sizes the viewer from these.
  return { path: outputPath, width: clip.width * scale, height: clip.height * scale };
}

if (!targetUrl) {
  console.log(JSON.stringify({ ok: false, error: 'missing URL argument' }));
  process.exit(2);
}

try {
  const result = await extractPage(targetUrl);
  console.log(
    JSON.stringify({
      ok: Boolean(result.ok),
      final_url: firstNonEmpty(result.finalUrl, targetUrl),
      title: cleanText(result.title, 4000),
      description: cleanText(result.description, 4000),
      author: cleanText(result.author, 4000),
      image_url: cleanText(result.imageUrl, 4000),
      content_text: cleanText(result.contentText),
      content_html: result.contentHtml || '',
      extractor: cleanText(result.extractor, 200),
      snapshot_width: result.snapshot?.width ?? null,
      snapshot_height: result.snapshot?.height ?? null,
      snapshot_error: result.snapshotError || null,
      error: result.error || null
    })
  );
} catch (error) {
  console.log(
    JSON.stringify({
      ok: false,
      error: error instanceof Error ? error.message : String(error)
    })
  );
  process.exit(1);
}
