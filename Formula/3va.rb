class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.9.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.9.0/3va-v2.9.0-x86_64-apple-darwin.tar.gz"
      sha256 "9697855d5f991888fbb091fc9f884a138d44eaeaa6618bd69bcc13e1e6a3983f"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.9.0/3va-v2.9.0-aarch64-apple-darwin.tar.gz"
      sha256 "6dac87b17cedfd4c3ffb8693cb4cd54bca177ff23b0a451b9ef649bd83afb76a"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.9.0/3va-v2.9.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "e14a2267ac517b8456389f64a1469fe7febe6e9e63b43c0a0984ff194aa6099b"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.9.0/3va-v2.9.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "12810b5552a58f7aad47619dda05b4fa4acd3094569fd6cbe65553bd14724cc7"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
