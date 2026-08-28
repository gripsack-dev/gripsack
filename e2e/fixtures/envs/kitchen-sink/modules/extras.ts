import { fileFetch, module, verifyDeployed } from "@gripsack/core";

export default module("extras", {
  fetch: fileFetch("payloads/x.tar.gz"),
  verify: verifyDeployed("~/.config/demo/a.toml"),
});
