import { chromium } from 'playwright-core';
import { existsSync } from 'node:fs';

const targetUrl = process.argv[2];

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
    const page = await browser.newPage({
      userAgent:
        'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36',
      viewport: { width: 1365, height: 900 },
      locale: 'zh-CN'
    });
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
    await page.waitForTimeout(6000);

    return await page.evaluate(() => {
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
        if (!detailMap || typeof detailMap !== 'object') return null;
        for (const detail of Object.values(detailMap)) {
          const note = detail?.note;
          if (note && (note.title || note.desc)) return note;
        }
        return null;
      }

      const ld = jsonLd();
      const xhsNote = xhsNoteFromState();
      if (xhsNote) {
        const image = xhsNote.imageList?.[0]?.urlDefault || xhsNote.imageList?.[0]?.urlPre || '';
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
        extractor: 'browser-render'
      };
    });
  } finally {
    await browser.close();
  }
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
      extractor: cleanText(result.extractor, 200)
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
