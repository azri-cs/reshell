class Rsh < Formula
  desc "Resilient Shell Execution Middleware for AI Agents"
  homepage "https://github.com/azri-cs/reshell"
  url "https://github.com/azri-cs/reshell/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "REPLACE_WITH_ACTUAL_SHA256_AFTER_RELEASE"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    output = shell_output("#{bin}/rsh --version")
    assert_match "rsh", output
    system "#{bin}/rsh", "exec", "--command", "echo hello"
  end
end
