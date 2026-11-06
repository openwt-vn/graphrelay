# TEMPLATE, not publishable yet. Copy into a tap (e.g. openwt-vn/homebrew-tap/Formula/graphrelay.rb) after:
#   1. the repository is public,
#   2. a release exists with archives named graphrelay-<tag>-<target>.tar.gz and matching .sha256 files,
#   3. VERSION and the sha256 values below are replaced with the real ones (cat the release's .sha256 files).
class Graphrelay < Formula
  desc "Graph-aware code search for AI coding agents (MCP, REST, web UI); one local binary"
  homepage "https://github.com/openwt-vn/graphrelay"
  license "MIT"
  version "VERSION" # e.g. "0.2.0"; archives are named with the tag, so keep the "v" prefix below consistent

  on_macos do
    on_arm do
      url "https://github.com/openwt-vn/graphrelay/releases/download/vVERSION/graphrelay-vVERSION-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256"
    end
    # x86_64-apple-darwin: the installer expects this target but no release has shipped it yet.
  end

  on_linux do
    on_intel do
      url "https://github.com/openwt-vn/graphrelay/releases/download/vVERSION/graphrelay-vVERSION-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_SHA256"
    end
    on_arm do
      url "https://github.com/openwt-vn/graphrelay/releases/download/vVERSION/graphrelay-vVERSION-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_SHA256"
    end
  end

  def install
    bin.install "gr"
  end

  test do
    assert_match "gr", shell_output("#{bin}/gr --help")
  end
end
