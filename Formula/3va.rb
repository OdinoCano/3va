class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.1.1"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.1/3va-v2.1.1-x86_64-apple-darwin.tar.gz"
      sha256 "e0d9e7d599acb721c66a637baf88636ded87e8c8ca06fd5a2520bb51b19c0233"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.1/3va-v2.1.1-aarch64-apple-darwin.tar.gz"
      sha256 "4042e4ed99d3c83060285fd0a45ab1d5dc7faf995341b3568b4352e95573fa70"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.1/3va-v2.1.1-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "271ec598ef6b9a8cc231e8ff73d3a49fe8b5b478be04b8a2ca020d53515ae1d9"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.1.1/3va-v2.1.1-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "3804ec43dc3f6bec81c86e362fa360535ba6a8fd7b73aaf3bbb8ea018778ee3d"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
