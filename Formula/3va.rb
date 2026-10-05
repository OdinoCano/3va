class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.12.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.0/3va-v2.12.0-x86_64-apple-darwin.tar.gz"
      sha256 "79844f21395c45fc3d63c388b6d311d021267df71673c384cf332efb07fed930"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.0/3va-v2.12.0-aarch64-apple-darwin.tar.gz"
      sha256 "8bf3b41b280c00af3db53b4383714bfc54ecba6208c8b81cb468a6d416f9a14d"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.0/3va-v2.12.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "7455119899e0a246e92242a1b40c24724d1a03402dbc1aab436a569e3f7d9255"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.0/3va-v2.12.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "3432ed4e37cc159a596fe78bdadb402514072f057fcea8f69f3091334176e00c"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
