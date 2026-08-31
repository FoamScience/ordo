# Homebrew formula — fill in sha256 values after cutting a release.
class Ordo < Formula
  desc "Comprehension-optimized ordering of code-change hunks"
  homepage "https://github.com/elwardi/ordo"
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/elwardi/ordo/releases/download/v0.2.0/ordo-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_ARM64_SHA256"
    end
    on_intel do
      url "https://github.com/elwardi/ordo/releases/download/v0.2.0/ordo-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X86_64_SHA256"
    end
  end

  def install
    bin.install "ordo"
  end

  test do
    assert_match "usage", shell_output("#{bin}/ordo --help 2>&1", 0)
  end
end
