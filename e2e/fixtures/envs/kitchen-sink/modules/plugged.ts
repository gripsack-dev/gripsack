import { module, pluginFetch, verifyFile } from "@gripsack/core";

export default module("plugged", {
  fetch: pluginFetch("apt", { package: "htop", version: "3.3.0" }),
  verify: verifyFile("bin/htop"),
});
