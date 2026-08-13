using System.Collections.Generic;
using System.IO;
using System.Xml.Linq;
using System.Xml.XPath;
using log4net;

namespace common.resources
{
    public class AppSettings : InitSettings
    {
        static ILog log = LogManager.GetLogger(typeof(AppSettings));

        public string MenuMusic { get; private set; }
        public string DeadMusic { get; private set; }
        public int EditorMinRank { get; private set; }
        public int CharacterSlotCost { get; private set; }
        public int CharacterSlotCurrency { get; private set; }
        public int VaultChestCost { get; private set; }

        /// <summary>
        /// The most chests one account may own.
        /// </summary>
        /// <remarks>
        /// This used to be decided by the map: the vault placed one chest per Vault-region tile and
        /// stopped when it ran out of floor. The chests are not in a world any more, so the limit
        /// has to be written down somewhere, and the panel needs to know it to stop offering more.
        /// </remarks>
        public int MaxVaultChests { get; private set; }

        /// <summary>
        /// How many chests every account has without paying for one.
        /// </summary>
        /// <remarks>
        /// Four, which is thirty-two slots. One was the old default and it was a hangover from the
        /// chests being objects on a map: a new account arrived in a room full of chests it did not
        /// own and could store eight things. The vault is a grid now and a grid with one row in it
        /// looks broken rather than empty.
        /// </remarks>
        public int FreeVaultChests { get; private set; }

        public int InventorySize { get; private set; }
        public int MaxStackablePotions { get; private set; }
        public int PotionPurchaseCooldown { get; private set; }
        public int PotionPurchaseCostCooldown { get; private set; }
        public int[] PotionPurchaseCosts { get; private set; }
        public bool DisableRegistration { get; private set; }
        public NewAccounts Accounts { get; private set; }
        public NewCharacters Characters { get; private set; }

        public AppSettings(string path)
        {
            log.Info("Loading app settings...");

            elem = XElement.Parse(File.ReadAllText(path));

            MenuMusic = GetStringValue("MenuMusic");
            DeadMusic = GetStringValue("DeadMusic");
            EditorMinRank = GetIntValue("EditorMinRank");
            CharacterSlotCost = GetIntValue("CharacterSlotCost");
            CharacterSlotCurrency = GetIntValue("CharacterSlotCurrency");
            VaultChestCost = GetIntValue("VaultChestCost");
            MaxStackablePotions = GetIntValue("MaxStackablePotions");
            PotionPurchaseCooldown = GetIntValue("PotionPurchaseCooldown");
            PotionPurchaseCostCooldown = GetIntValue("PotionPurchaseCostCooldown");
            DisableRegistration = GetBoolValue("DisableRegist");
            Accounts = new NewAccounts(elem.Element("NewAccounts"));
            Characters = new NewCharacters(elem.Element("NewCharacters"));

            InventorySize = GetIntValue("InventorySize");
            if (InventorySize == 0) InventorySize = 24;

            MaxVaultChests = GetIntValue("MaxVaultChests");
            if (MaxVaultChests <= 0) MaxVaultChests = 40;

            FreeVaultChests = GetIntValue("FreeVaultChests");
            if (FreeVaultChests <= 0) FreeVaultChests = 4;
            if (FreeVaultChests > MaxVaultChests) FreeVaultChests = MaxVaultChests;

            if (Exists("PotionPurchaseCosts"))
            {
                var potCosts = elem.Element("PotionPurchaseCosts");
                var potCostList = new List<int>();
                foreach (var e in potCosts.XPathSelectElements("//cost"))
                {
                    int cost = 0;
                    int.TryParse(e.Value, out cost);
                    potCostList.Add(cost);
                }
                PotionPurchaseCosts = potCostList.ToArray();
            }
        }
    }

    public class NewAccounts : InitSettings
    {
        public int Gold { get; private set; }
        public int Fame { get; private set; }
        public int Prestige { get; private set; }
        public bool ClassesUnlocked { get; private set; }
        public bool SkinsUnlocked { get; private set; }
        public int VaultCount { get; private set; }
        public int MaxCharSlot { get; private set; }

        public NewAccounts(XElement e)
        {
            elem = e;

            Gold = GetIntValue("Gold");
            Fame = GetIntValue("Fame");
            Prestige = GetIntValue("Prestige");
            ClassesUnlocked = GetBoolValue("ClassesUnlocked");
            SkinsUnlocked = GetBoolValue("SkinsUnlocked");
            VaultCount = GetIntValue("VaultCount");
            MaxCharSlot = GetIntValue("MaxCharSlot");
        }
    }

    public class NewCharacters : InitSettings
    {
        public bool Maxed { get; private set; }
        public int Level { get; private set; }

        public NewCharacters(XElement e)
        {
            elem = e;

            Maxed = GetBoolValue("Maxed");
            Level = GetIntValue("Level");
        }
    }

    public class InitSettings
    {
        protected XElement elem;

        protected bool Exists(string element)
        {
            return elem.Element(element) != null;
        }

        protected int GetIntValue(string element)
        {
            return Exists(element) ? int.Parse(elem.Element(element).Value) : 0;
        }

        protected bool GetBoolValue(string element)
        {
            return Exists(element) ? (elem.Element(element).Value.Equals("1")) : false;
        }

        protected string GetStringValue(string element)
        {
            return Exists(element) ? elem.Element(element).Value : "";
        }
    }
}
