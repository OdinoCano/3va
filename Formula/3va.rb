class ThreeVa < Formula
  desc "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally."
  homepage "https://github.com/OdinoCano/3va"
  license "MIT"
  version "2.12.6"

  on_macos do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.6/3va-v2.12.6-x86_64-apple-darwin.tar.gz"
      sha256 "b253c91759e6f8a054940e281edc6ee7e8ef542c16d5dbd0f6f19437fbfa6449"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.6/3va-v2.12.6-aarch64-apple-darwin.tar.gz"
      sha256 "1dfffd4e9ea95eec37ef7013b8cfe7c54386a52b48bf4f59677ac393c2c2b212"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.6/3va-v2.12.6-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "57a67b848393eda94bc36f223332caf780ea213d11f8e54addac7393e22392e8"
    end
    on_arm do
      url "https://github.com/OdinoCano/3va/releases/download/v2.12.6/3va-v2.12.6-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "17d2960b0d492b5ae35830537de941ffc82dbaf9f04293a05aa3d9ad7dd91167"
    end
  end

  def install
    bin.install "3va"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/3va --version")
  end
end
