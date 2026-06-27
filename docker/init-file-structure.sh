
mkdir -p data/{home,config,cache,runtime}
mkdir -p data/home/{.codex,.gemini,.antigravity_cockpit}

sudo chown -R $(id -u):$(id -g) data
