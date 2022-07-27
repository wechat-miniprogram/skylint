import { readFile } from "fs/promises";
import { defineRule, RuleType } from "src/rules/interface";
import { isType } from "src/walker/css";
import { BasicParseEnv, parse } from "src/parser";
import { existsSync } from "fs";
import { resolvePath } from "./resolve";

interface RuleEnv extends BasicParseEnv {
  rootPath: string;
  wxssPaths: string[];
  wxssSet: Set<string>;
}

/**
 * @param wxssPaths accept absoulte pathes of wxss file
 */
export const collectImportedWXSS = async (wxssPaths: string[], rootPath: string) => {
  const originPaths = wxssPaths.slice();
  const wxssSet = new Set(wxssPaths);

  const rule = defineRule<RuleEnv, RuleType.WXSS>({ name: "collect-imported-wxss", type: RuleType.WXSS }, (ctx) => {
    ctx.lifetimes({
      onVisit: (node) => {
        if (
          !isType(node, "Atrule") ||
          node.name !== "import" ||
          !node.prelude ||
          !isType(node.prelude, "AtrulePrelude")
        ) {
          return;
        }
        const { path: currentPath, wxssPaths, wxssSet, rootPath } = ctx.env!;
        node.prelude.children.forEach((child) => {
          // type `String` for `import "style.wxss"`
          let path: string | null = null;
          if (isType(child, "String")) {
            // type `String` for `import "style.wxss"`
            path = child.value;
          } else if (isType(child, "Url") && isType(child.value, "String")) {
            // type `Url` for `import url("style.wxss")`
            path = child.value.value;
          }
          if (!path?.endsWith(".wxss")) return;
          path = resolvePath(currentPath, rootPath, path);

          if (!existsSync(path) || wxssSet.has(path)) return;
          wxssSet.add(path);
          wxssPaths.push(path);
        });
      },
    });
  });

  for (const wxssPath of wxssPaths) {
    const wxss = (await readFile(wxssPath)).toString();
    const env = { rootPath, path: wxssPath, wxssSet, wxssPaths };
    parse({ wxss, Rules: [rule], env });
  }

  for (const wxssPath of originPaths) {
    wxssSet.delete(wxssPath); // remove non-imported wxss
  }
  return wxssSet;
};
