import { LocationLnColBased, SourceCodeLocation } from "src/rules/interface";
import { codeFrameColumns } from "@babel/code-frame";
import lineColumn from "line-column";
import { format } from "util";

interface PrintCodeOptions {
  /** @default false */
  withCodeFrame?: boolean;
  alternativeFilename?: string;
}

export const formatSourceCodeLocation = (rawStr: string, loc: SourceCodeLocation, options: PrintCodeOptions = {}) => {
  const { withCodeFrame = false, alternativeFilename } = options;
  let ret = "";
  let location: LocationLnColBased;
  if ("startCol" in loc) {
    location = loc;
  } else {
    const finder = lineColumn(rawStr);
    const { line: startLn = -1, col: startCol = -1 } = finder.fromIndex(loc.startIndex) ?? {};
    const { line: endLn = -1, col: endCol = -1 } = finder.fromIndex(loc.endIndex) ?? {};
    location = { startLn, startCol, endLn, endCol, path: loc.path };
  }

  const filenameWithLnCol = format("%s:%d:%d", loc.path ?? alternativeFilename, location.startLn, location.startCol);
  if (!withCodeFrame) return filenameWithLnCol;

  const codeFrame = codeFrameColumns(
    rawStr,
    {
      start: {
        line: location.startLn,
        column: location.startCol,
      },
      end: {
        line: location.endLn,
        column: location.endCol,
      },
    },
    {
      linesAbove: 1,
      linesBelow: 1,
      forceColor: true,
    }
  );

  if (codeFrame.length > 256) return filenameWithLnCol;

  return [filenameWithLnCol, codeFrame].join("\n");
};
