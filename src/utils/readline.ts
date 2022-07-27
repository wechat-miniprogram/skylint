import { createInterface } from "readline";
import { stdin as input, stdout as output } from "node:process";
import { promisify } from "node:util";

export const question = (query: string) => {
  const rl = createInterface({ input, output });
  return promisify(rl.question)(query);
};
