import { Fragment, type ReactNode } from "react";

type MarkdownMessageProps = {
  content: string;
};

const unorderedItem = /^\s*[-+*]\s+(.+)$/;
const orderedItem = /^\s*\d+[.)]\s+(.+)$/;
const fencedCode = /^\s*```([\w.+-]*)\s*$/;
const heading = /^(#{1,6})\s+(.+)$/;
const quote = /^\s*>\s?(.*)$/;

export function MarkdownMessage({ content }: MarkdownMessageProps) {
  const lines = content.replace(/\r\n?/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    if (!line.trim()) {
      index += 1;
      continue;
    }

    const fence = line.match(fencedCode);
    if (fence) {
      const language = fence[1];
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !fencedCode.test(lines[index])) {
        code.push(lines[index]);
        index += 1;
      }
      if (index < lines.length) index += 1;
      blocks.push(
        <pre className="markdown-code-block" key={`code-${blocks.length}`}>
          <code className={language ? `language-${language}` : undefined}>{code.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    const headingMatch = line.match(heading);
    if (headingMatch) {
      blocks.push(renderHeading(headingMatch[1].length, inlineMarkdown(headingMatch[2], `heading-${blocks.length}`), `heading-${blocks.length}`));
      index += 1;
      continue;
    }

    if (isHorizontalRule(line)) {
      blocks.push(<hr key={`rule-${blocks.length}`} />);
      index += 1;
      continue;
    }

    if (isTableHeader(lines, index)) {
      const headers = splitTableRow(line);
      const rows: string[][] = [];
      index += 2;
      while (index < lines.length && lines[index].includes("|") && lines[index].trim()) {
        rows.push(splitTableRow(lines[index]));
        index += 1;
      }
      blocks.push(
        <div className="markdown-table-wrap" key={`table-${blocks.length}`}>
          <table>
            <thead>
              <tr>{headers.map((cell, cellIndex) => <th key={cellIndex}>{inlineMarkdown(cell, `th-${cellIndex}`)}</th>)}</tr>
            </thead>
            <tbody>
              {rows.map((row, rowIndex) => (
                <tr key={rowIndex}>
                  {headers.map((_, cellIndex) => <td key={cellIndex}>{inlineMarkdown(row[cellIndex] ?? "", `td-${rowIndex}-${cellIndex}`)}</td>)}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }

    const quoteMatch = line.match(quote);
    if (quoteMatch) {
      const quoted = [quoteMatch[1]];
      index += 1;
      while (index < lines.length) {
        const nextQuote = lines[index].match(quote);
        if (!nextQuote) break;
        quoted.push(nextQuote[1]);
        index += 1;
      }
      blocks.push(<blockquote key={`quote-${blocks.length}`}>{inlineMarkdown(quoted.join(" "), `quote-${blocks.length}`)}</blockquote>);
      continue;
    }

    const unorderedMatch = line.match(unorderedItem);
    if (unorderedMatch) {
      const items = [unorderedMatch[1]];
      index += 1;
      while (index < lines.length) {
        const nextItem = lines[index].match(unorderedItem);
        if (!nextItem) break;
        items.push(nextItem[1]);
        index += 1;
      }
      blocks.push(
        <ul key={`ul-${blocks.length}`}>
          {items.map((item, itemIndex) => <li key={itemIndex}>{inlineMarkdown(item, `ul-${itemIndex}`)}</li>)}
        </ul>,
      );
      continue;
    }

    const orderedMatch = line.match(orderedItem);
    if (orderedMatch) {
      const items = [orderedMatch[1]];
      index += 1;
      while (index < lines.length) {
        const nextItem = lines[index].match(orderedItem);
        if (!nextItem) break;
        items.push(nextItem[1]);
        index += 1;
      }
      blocks.push(
        <ol key={`ol-${blocks.length}`}>
          {items.map((item, itemIndex) => <li key={itemIndex}>{inlineMarkdown(item, `ol-${itemIndex}`)}</li>)}
        </ol>,
      );
      continue;
    }

    const paragraph = [line.trim()];
    index += 1;
    while (index < lines.length && lines[index].trim() && !startsBlock(lines, index)) {
      paragraph.push(lines[index].trim());
      index += 1;
    }
    blocks.push(<p key={`paragraph-${blocks.length}`}>{inlineMarkdown(paragraph.join(" "), `paragraph-${blocks.length}`)}</p>);
  }

  return <div className="markdown-content">{blocks}</div>;
}

function startsBlock(lines: string[], index: number) {
  const line = lines[index];
  return fencedCode.test(line)
    || heading.test(line)
    || isHorizontalRule(line)
    || unorderedItem.test(line)
    || orderedItem.test(line)
    || quote.test(line)
    || isTableHeader(lines, index);
}

function isHorizontalRule(line: string) {
  return /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line);
}

function splitTableRow(line: string) {
  let normalized = line.trim();
  if (normalized.startsWith("|")) normalized = normalized.slice(1);
  if (normalized.endsWith("|")) normalized = normalized.slice(0, -1);
  return normalized.split("|").map((cell) => cell.trim());
}

function isTableHeader(lines: string[], index: number) {
  if (index + 1 >= lines.length || !lines[index].includes("|")) return false;
  const separators = splitTableRow(lines[index + 1]);
  return separators.length > 0 && separators.every((cell) => /^:?-{3,}:?$/.test(cell));
}

function renderHeading(level: number, children: ReactNode, key: string) {
  switch (level) {
    case 1: return <h1 key={key}>{children}</h1>;
    case 2: return <h2 key={key}>{children}</h2>;
    case 3: return <h3 key={key}>{children}</h3>;
    case 4: return <h4 key={key}>{children}</h4>;
    case 5: return <h5 key={key}>{children}</h5>;
    default: return <h6 key={key}>{children}</h6>;
  }
}

function inlineMarkdown(value: string, keyPrefix: string): ReactNode[] {
  const tokens = value.split(/(`[^`\n]+`|\*\*[^*\n]+\*\*|__[^_\n]+__|~~[^~\n]+~~|\[[^\]\n]+\]\([^()\s]+\)|\*[^*\n]+\*|_[^_\n]+_)/g);
  return tokens.filter(Boolean).map((token, index) => {
    const key = `${keyPrefix}-${index}`;
    if (token.startsWith("`") && token.endsWith("`")) return <code key={key}>{token.slice(1, -1)}</code>;
    if ((token.startsWith("**") && token.endsWith("**")) || (token.startsWith("__") && token.endsWith("__"))) {
      return <strong key={key}>{inlineMarkdown(token.slice(2, -2), `${key}-strong`)}</strong>;
    }
    if (token.startsWith("~~") && token.endsWith("~~")) return <del key={key}>{token.slice(2, -2)}</del>;
    if ((token.startsWith("*") && token.endsWith("*")) || (token.startsWith("_") && token.endsWith("_"))) {
      return <em key={key}>{token.slice(1, -1)}</em>;
    }
    const link = token.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
    if (link) {
      const href = safeHref(link[2]);
      if (!href) return <Fragment key={key}>{token}</Fragment>;
      const external = /^https?:\/\//i.test(href);
      return <a href={href} key={key} rel={external ? "noreferrer noopener" : undefined} target={external ? "_blank" : undefined}>{link[1]}</a>;
    }
    return <Fragment key={key}>{token}</Fragment>;
  });
}

function safeHref(value: string) {
  const href = value.trim();
  return /^(?:https?:\/\/|mailto:|\/|#)/i.test(href) ? href : null;
}
