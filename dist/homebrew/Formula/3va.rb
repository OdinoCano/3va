class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.0.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "8241b8615cb7802e6740c035cc081400e33d17dc812846623d092ce9c25ff3ed"
    end

    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "af0f3deb5187e551fab84062d53d86d8618ab126dc2f7a47215c5addc6b82241"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "ddfd46aee3b0b86d448c7fa5e94ae902b28acfb707089db17a53720e2521f27f"
    end

    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.0.0/3va-v2.0.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "1d825a34203ed2d9d16bbdea7daa74644a5b29bb0df63602b00bb1801b968f6d"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
