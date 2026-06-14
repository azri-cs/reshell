class Reshell < Formula
  desc "Resilient Shell Execution Middleware for AI Agents"
  homepage "https://github.com/azri-cs/reshell"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/azri-cs/reshell/releases/download/v#{version}/rsh-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_AARCH64_DARWIN_SHA256"
    end
    on_intel do
      url "https://github.com/azri-cs/reshell/releases/download/v#{version}/rsh-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X86_64_DARWIN_SHA256"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/azri-cs/reshell/releases/download/v#{version}/rsh-aarch64-unknown-linux-musl.tar.gz"
      sha256 "REPLACE_WITH_AARCH64_LINUX_SHA256"
    end
    on_intel do
      url "https://github.com/azri-cs/reshell/releases/download/v#{version}/rsh-x86_64-unknown-linux-musl.tar.gz"
      sha256 "REPLACE_WITH_X86_64_LINUX_SHA256"
    end
  end

  def install
    bin.install "rsh"
  end

  test do
    assert_match "Resilient Shell Execution Middleware", shell_output("#{bin}/rsh --version")
  end
end
