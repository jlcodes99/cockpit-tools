package helps

import (
	"strings"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"gopkg.in/yaml.v3"
)

func TestRetiredAPIServiceConfigDoesNotReactivatePolicy(t *testing.T) {
	var cfg config.Config
	if err := yaml.Unmarshal([]byte("codex:\n  api-service-compatibility: true\n  stream-bootstrap-buffering: true\n"), &cfg); err != nil {
		t.Fatal(err)
	}
	if cfg.Codex.IdentityConfuse || !cfg.Codex.StreamBootstrapBuffering {
		t.Fatal("legacy config changed identity policy or lost upstream buffering")
	}
	encoded, err := yaml.Marshal(cfg.Codex)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(encoded), "api-service-compatibility") {
		t.Fatal("retired capacity policy was serialized again")
	}
}
