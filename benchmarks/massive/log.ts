import * as fs from "fs";
import * as path from "path";
import * as sax from "sax";
import wtf from "wtf_wikipedia";

const FILE_PATH = path.join(
  __dirname,
  "enwiki-2026-06-01-p1130125p3931711.xml",
);

interface CleanArticle {
  id: string;
  title: string;
  body: string;
}

function parseWikiWithLibrary(filePath: string, targetCount: number = 10) {
  if (!fs.existsSync(filePath)) {
    console.error(`File not found: ${filePath}`);
    return;
  }

  const fileStream = fs.createReadStream(filePath, { encoding: "utf8" });
  const saxStream = sax.createStream(true, { lowercase: true, trim: true });

  let loggedCount = 0;
  let currentTag = "";
  let inPage = false;
  let inRevision = false;

  let pageId = "";
  let pageTitle = "";
  let pageBody = "";
  let namespace = "";

  saxStream.on("opentag", (node) => {
    currentTag = node.name;
    if (currentTag === "page") {
      inPage = true;
      pageId = "";
      pageTitle = "";
      pageBody = "";
      namespace = "";
    } else if (currentTag === "revision") {
      inRevision = true;
    }
  });

  saxStream.on("text", (text) => {
    if (!inPage) return;

    switch (currentTag) {
      case "title":
        pageTitle += text;
        break;
      case "ns":
        namespace += text;
        break;
      case "id":
        if (!inRevision) pageId += text;
        break;
      case "text":
        pageBody += text;
        break;
    }
  });

  saxStream.on("closetag", (tagName) => {
    if (tagName === "revision") {
      inRevision = false;
    }

    if (tagName === "page") {
      inPage = false;

      if (namespace === "0" && pageTitle) {
        loggedCount++;

        let cleanBody = "";
        try {
          // 1. Extract pure plaintext using the library
          cleanBody = wtf(pageBody).text();

          // 2. Remove all newlines and replace them with a space
          cleanBody = cleanBody.replace(/\n+/g, " ");

          // 3. Collapse any resulting double spaces down to a single space
          cleanBody = cleanBody.replace(/\s+/g, " ").trim();
        } catch (err) {
          cleanBody = "[Error parsing text component]";
        }

        // Log as raw, unescaped plain text for maximum human readability
        console.log(`\n==============================================`);
        console.log(`=== Cleaned Article #${loggedCount} ===`);
        console.log(`==============================================`);
        console.log(`ID:    ${pageId}`);
        console.log(`TITLE: ${pageTitle}`);
        console.log(`BODY:  ${cleanBody}`);
        console.log(`\n`);

        if (loggedCount >= targetCount) {
          console.log(`Reached target of ${targetCount}. Halting stream.`);
          fileStream.destroy();
        }
      }
    }

    if (currentTag === tagName) {
      currentTag = "";
    }
  });

  fileStream.pipe(saxStream);
}

parseWikiWithLibrary(FILE_PATH, 10);
