import { dirname, join, resolve } from "path";

export const resolvePath = (currentPath: string, rootPath: string, filePath: string) => {
  let path = "";
  if (filePath.startsWith("/")) {
    path = join(rootPath, filePath);
  } else {
    path = resolve(dirname(currentPath), filePath);
  }
  return path;
};
