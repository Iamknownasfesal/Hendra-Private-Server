using System;
using wServer.realm.entities;

namespace wServer.realm
{
    public enum StatsType : byte
    {
        MaximumHP = 0,
        HP = 1,
        Size = 2,
        MaximumMP = 3,
        MP = 4,
        ExperienceGoal = 5,
        Experience = 6,
        Level = 7,
        Inventory0 = 8,
        Inventory1 = 9,
        Inventory2 = 10,
        Inventory3 = 11,
        Inventory4 = 12,
        Inventory5 = 13,
        Inventory6 = 14,
        Inventory7 = 15,
        Inventory8 = 16,
        Inventory9 = 17,
        Inventory10 = 18,
        Inventory11 = 19,
        Inventory12 = 20,
        Inventory13 = 21,
        Inventory14 = 22,
        Inventory15 = 23,
        Attack = 24,
        Defense = 25,
        Speed = 26,
        Vitality = 27,
        Wisdom = 28,
        Dexterity = 29,
        Effects = 30,
        Stars = 31,
        Name = 32,
        Texture1 = 33,
        Texture2 = 34,
        MerchantMerchandiseType = 35,
        Credits = 36,
        SellablePrice = 37,
        PortalUsable = 38,
        AccountId = 39,
        CurrentFame = 40,
        SellablePriceCurrency = 41,
        ObjectConnection = 42,
        MerchantRemainingCount = 43,
        MerchantRemainingMinute = 44,
        MerchantDiscount = 45,
        SellableRankRequirement = 46,
        HPBoost = 47,
        MPBoost = 48,
        AttackBonus = 49,
        DefenseBonus = 50,
        SpeedBonus = 51,
        VitalityBonus = 52,
        WisdomBonus = 53,
        DexterityBonus = 54,
        OwnerAccountId = 55,
        NameChangerStar = 56,
        NameChosen = 57,
        Fame = 58,
        FameGoal = 59,
        Glow = 60,
        SinkOffset = 61,
        AltTextureIndex = 62,
        Guild = 63,
        GuildRank = 64,
        OxygenBar = 65,
        XPBoost = 66,
        XPBoostTime = 67,
        LDBoostTime = 68,
        LTBoostTime = 69,
        HealthStackCount = 70,
        MagicStackCount = 71,
        BackPack0 = 72,
        BackPack1 = 73,
        BackPack2 = 74,
        BackPack3 = 75,
        BackPack4 = 76,
        BackPack5 = 77,
        BackPack6 = 78,
        BackPack7 = 79,
        HasBackpack = 80,
        Skin = 81,
        Effects2 = 82,
        DamageMin = 83,
        DamageMax = 84,
        DamageMinBonus = 85,
        DamageMaxBonus = 86,
        LuckBonus = 87,
        Rank = 88,
        Admin = 89,
        Luck = 90,
        Prestige = 91,

        None = 255
    }

    public class SV<T>
    {
        private readonly Entity _owner;
        private readonly StatsType _type;
        private readonly bool _updateSelfOnly;
        private readonly Func<T, T> _transform;
        private T _value;
        private T _tValue;
        
        public SV(Entity e, StatsType type, T value, bool updateSelfOnly = false, Func<T,T> transform = null)
        {
            _owner = e;
            _type = type;
            _updateSelfOnly = updateSelfOnly;
            _transform = transform;

            _value = value;
            _tValue = Transform(value);
        }

        public T GetValue()
        {
            return _value;
        }

        public void SetValue(T value)
        {
            if (_value != null && _value.Equals(value))
                return;
            _value = value;

            var tVal = Transform(value);
            if (_tValue != null && _tValue.Equals(tVal))
                return;
            _tValue = tVal;

            // hacky fix to xp
            if (_owner is Player && _type == StatsType.Experience)
            {
                _owner.InvokeStatChange(_type, (int)(object) tVal - Player.GetLevelExp((_owner as Player).Level), _updateSelfOnly);
            }
            else
            {
                _owner.InvokeStatChange(_type, tVal, _updateSelfOnly);
            }
        }

        public override string ToString()
        {
            return _value.ToString();
        }

        private T Transform(T value)
        {
            return (_transform == null) ? value : _transform(value);
        }
    }

    public class StatChangedEventArgs : EventArgs
    {
        public StatChangedEventArgs(StatsType stat, object value, bool updateSelfOnly = false)
        {
            Stat = stat;
            Value = value;
            UpdateSelfOnly = updateSelfOnly;
        }

        public StatsType Stat { get; private set; }
        public object Value { get; private set; }
        public bool UpdateSelfOnly { get; private set; }
    }
}
