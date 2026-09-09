class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.8.0"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.8.0/3va-v2.8.0-x86_64-apple-darwin.tar.gz"
      sha256 "5e97edac03c1fcffe1ed4845f88b2d46ea1e3c50c72c2c3b9d3f05fb4e463fbc"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.8.0/3va-v2.8.0-aarch64-apple-darwin.tar.gz"
      sha256 "16a2c32ff2dd75083d5a4d144c9796086dfb01e4e1e62168ba698beb55c5b29f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.8.0/3va-v2.8.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "39f643ad054bc3b035fdae9d969011e5fc1d79210af4445709de0e4b3e2d68d6"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.8.0/3va-v2.8.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "382e5e4434c3dee3e781e8a05718c4f680fba2cd2f051354224d9e0722348180"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
