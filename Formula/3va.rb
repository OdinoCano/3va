class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.8.1"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.8.1/3va-v2.8.1-x86_64-apple-darwin.tar.gz"
      sha256 "091f75dc1ab013322ba1fde60f6218996f112b66237ed72fdd125f480963a159"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.8.1/3va-v2.8.1-aarch64-apple-darwin.tar.gz"
      sha256 "c5930894457b1256a2232431532688ad2bdfdb60d586593a95ad574ffbd62cc0"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.8.1/3va-v2.8.1-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "50e547eb63deef39e3748e8112d893e73e9398b6dda83b452126639fd5dc9cb4"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.8.1/3va-v2.8.1-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "1e431cd16e09b6f880ff3e47afdd430619b5d3b6747cf800cccbb015eecb81cf"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
