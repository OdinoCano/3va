class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.6.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-x86_64-apple-darwin.tar.gz"
      sha256 "1343a06bd0dde5de19014dabe8573202b74ef588baab62767c0c25c990ea4e8c"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-aarch64-apple-darwin.tar.gz"
      sha256 "cd38872eeadd1d3a5237f1640fbbc1df9d293de4f22572e12e417651d14a115f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "c32f807beaa0b7a51cd5f3e0778db88068390f8c301530ac8f8aa8751eb2c669"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.6.0/3va-v2.6.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "777fa59930d3ce503afa7122119571fa8d8d24f4c6b4a28a1fd0c6bcfa6a8a47"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
