package mixed_tests

import (
	"context"
	"os"
	"slices"
	"testing"
	"time"

	"github.com/matrix-org/complement"
	dockerclient "github.com/moby/moby/client"
)

func TestMain(m *testing.M) {
	complement.TestMain(m, "palpo_mixed")
}

// Check the deployed containers, not just the runner's intended image mapping.
func TestMixedHomeserverImages(t *testing.T) {
	deployment := complement.Deploy(t, 3)
	defer deployment.Destroy(t)
	cli, err := dockerclient.NewClientWithOpts(dockerclient.FromEnv, dockerclient.WithAPIVersionNegotiation())
	if err != nil {
		t.Fatal(err)
	}
	defer cli.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	images := make(map[string]string)
	for _, name := range []string{"hs1", "hs2", "hs3"} {
		want := os.Getenv("COMPLEMENT_BASE_IMAGE_" + name)
		if want == "" {
			t.Fatalf("missing image mapping for %s", name)
		}
		container, err := cli.ContainerInspect(ctx, deployment.ContainerID(t, name), dockerclient.ContainerInspectOptions{})
		if err != nil {
			t.Fatal(err)
		}
		base, err := cli.ImageInspect(ctx, want)
		if err != nil {
			t.Fatal(err)
		}
		deployed, err := cli.ImageInspect(ctx, container.Container.Image)
		if err != nil {
			t.Fatal(err)
		}
		// Clean deployments commit a blueprint snapshot with a new image ID.
		// Verify its filesystem descends from the intended base image instead
		// of comparing the snapshot's untagged ID with the base's tag.
		layers := base.RootFS.Layers
		if len(layers) == 0 || len(deployed.RootFS.Layers) < len(layers) ||
			!slices.Equal(deployed.RootFS.Layers[:len(layers)], layers) {
			t.Fatalf("%s runs %s, which does not descend from %s (%s)", name, deployed.ID, want, base.ID)
		}
		t.Logf("%s runs %s derived from %s (%s)", name, deployed.ID, want, base.ID)
		images[name] = base.ID
	}
	if images["hs1"] == images["hs2"] {
		t.Fatal("mixed federation requires distinct hs1 and hs2 images")
	}
	if images["hs1"] != images["hs3"] {
		t.Fatal("hs3 must use the same image as hs1")
	}
}
